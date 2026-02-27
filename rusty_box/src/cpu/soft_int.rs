//! Software interrupt instructions for x86 CPU emulation
//!
//! Based on Bochs soft_int.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements INT, INT3, INTO, IRET instructions

use super::{
    cpu::{BxCpuC, CpuActivityState, BX_ASYNC_EVENT_STOP_TRACE},
    cpuid::BxCpuIdTrait,
    decoder::{Instruction, BxSegregs},
    descriptor::BxSelector,
    segment_ctrl_pro::parse_selector,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // INT - Software Interrupt
    // =========================================================================
    
    /// INT imm8 - Software interrupt with immediate vector
    pub fn int_ib(&mut self, instr: &Instruction) {
        let vector = instr.ib();
        tracing::debug!("INT {:#04x}", vector);
        let _ = self.interrupt_real_mode(vector);
    }

    /// INT3 - Breakpoint interrupt (vector 3)
    pub fn int3(&mut self, _instr: &Instruction) {
        tracing::debug!("INT3 (breakpoint)");
        let _ = self.interrupt_real_mode(3);
    }

    /// INTO - Interrupt on overflow (vector 4, only if OF=1)
    pub fn into(&mut self, _instr: &Instruction) {
        if self.get_of() {
            tracing::debug!("INTO: overflow detected, calling INT 4");
            let _ = self.interrupt_real_mode(4);
        }
    }

    /// INT1 (ICEBP) - In-circuit emulator breakpoint (vector 1)
    pub fn int1(&mut self, _instr: &Instruction) -> super::Result<()> {
        tracing::warn!("INT1 (ICEBP) at RIP={:#x} CS={:#x}", self.rip(),
            self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize].selector.value);
        self.interrupt_real_mode(1)?;
        Ok(())
    }

    // =========================================================================
    // BOUND - Check Array Index Against Bounds
    // Based on Bochs soft_int.cc BOUND_GwMa and BOUND_GdMa
    // =========================================================================

    /// BOUND r16, m16&16 - Check 16-bit register against bounds in memory
    ///
    /// Compares the signed value in r16 against the signed lower and upper bounds
    /// at memory location. If the index is out of bounds, generates #BR exception.
    pub fn bound_gw_ma(&mut self, instr: &Instruction) -> super::Result<()> {
        // Get the 16-bit register value (signed)
        let op1_16 = self.get_gpr16(instr.dst() as usize) as i16;

        // Calculate effective address
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);

        // Read lower and upper bounds from memory (2 words)
        let bound_min = self.read_virtual_word(seg, eaddr)? as i16;
        let bound_max = self.read_virtual_word(seg, eaddr.wrapping_add(2))? as i16;

        tracing::trace!(
            "BOUND r16: value={}, min={}, max={}",
            op1_16, bound_min, bound_max
        );

        // Check if value is outside bounds
        if op1_16 < bound_min || op1_16 > bound_max {
            tracing::debug!("BOUND: fails bounds test (value {} not in [{}, {}])",
                op1_16, bound_min, bound_max);
            // Generate #BR exception (Bound Range Exceeded, vector 5)
            self.interrupt_real_mode(5)?;
        }
        Ok(())
    }

    /// BOUND r32, m32&32 - Check 32-bit register against bounds in memory
    ///
    /// Compares the signed value in r32 against the signed lower and upper bounds
    /// at memory location. If the index is out of bounds, generates #BR exception.
    pub fn bound_gd_ma(&mut self, instr: &Instruction) -> super::Result<()> {
        // Get the 32-bit register value (signed)
        let op1_32 = self.get_gpr32(instr.dst() as usize) as i32;

        // Calculate effective address
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);

        // Read lower and upper bounds from memory (2 dwords)
        let bound_min = self.read_virtual_dword(seg, eaddr)? as i32;
        let bound_max = self.read_virtual_dword(seg, eaddr.wrapping_add(4))? as i32;

        tracing::trace!(
            "BOUND r32: value={}, min={}, max={}",
            op1_32, bound_min, bound_max
        );

        // Check if value is outside bounds
        if op1_32 < bound_min || op1_32 > bound_max {
            tracing::debug!("BOUND: fails bounds test (value {} not in [{}, {}])",
                op1_32, bound_min, bound_max);
            // Generate #BR exception (Bound Range Exceeded, vector 5)
            self.interrupt_real_mode(5)?;
        }
        Ok(())
    }

    // =========================================================================
    // IRET - Interrupt Return
    // =========================================================================
    
    /// IRET - Return from interrupt (16-bit operand size)
    pub fn iret16(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Pop IP, CS, FLAGS from stack
        let new_ip = self.pop_16()?;
        let new_cs = self.pop_16()?;
        let new_flags = self.pop_16()?;
        
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
        Ok(())
    }

    /// IRET - Return from interrupt (32-bit operand size)
    pub fn iret32(&mut self, _instr: &Instruction) -> super::Result<()> {
        if !self.real_mode() {
            return self.iret_protected();
        }

        // Real mode: Pop EIP, CS, EFLAGS from stack
        let new_eip = self.pop_32()?;
        let new_cs = self.pop_32()? as u16;
        let new_eflags = self.pop_32()?;

        // Load CS with new selector (real mode: base = selector << 4)
        let cs_index = BxSegregs::Cs as usize;
        parse_selector(new_cs, &mut self.sregs[cs_index].selector);
        unsafe {
            self.sregs[cs_index].cache.u.segment.base = (new_cs as u64) << 4;
        }

        // Set EIP
        self.set_eip(new_eip);

        // Update EFLAGS (keep VM unchanged: 0x00257fd5 mask)
        self.eflags = (self.eflags & 0x00020000) | (new_eflags & !0x00020000u32);

        // Invalidate prefetch
        self.eip_fetch_ptr = None;
        self.eip_page_window_size = 0;

        tracing::debug!("IRET32: returning to {:04x}:{:08x}, eflags={:08x}", new_cs, new_eip, new_eflags);
        Ok(())
    }

    /// IRET in protected mode (32-bit operand size)
    ///
    /// Based on Bochs iret.cc:iret_protected() with os32=true.
    /// Reads EIP/CS/EFLAGS from stack WITHOUT advancing ESP first, then after all
    /// validation passes loads CS from the GDT (NOT real-mode segment << 4).
    fn iret_protected(&mut self) -> super::Result<()> {
        use super::{cpu::Exception, error::CpuError};

        // Nested Task (NT) — task-switch IRET; not needed for Linux kernel
        if (self.eflags & 0x0000_4000) != 0 {
            tracing::error!("iret_protected: NT flag set — nested task IRET not implemented");
            return Err(CpuError::UnimplementedOpcode {
                opcode: "IRET with NT flag (nested task)".to_string(),
            });
        }

        // Peek at stack without modifying ESP
        let temp_esp = if self.is_stack_32bit() {
            self.esp()
        } else {
            self.sp() as u32
        };

        let new_eip    = self.stack_read_dword(temp_esp + 0)?;
        let raw_cs_raw = self.stack_read_dword(temp_esp + 4)? as u16;
        let new_eflags = self.stack_read_dword(temp_esp + 8)?;

        // If VM bit is set in the saved EFLAGS and CPL==0, stack-return to V86 mode.
        // Not implemented; just log and continue (Linux never does this).
        if (new_eflags & 0x0002_0000) != 0 {
            tracing::warn!("iret_protected: VM bit set in saved EFLAGS — V86 return not implemented");
        }

        // Return CS selector must be non-null
        if (raw_cs_raw & 0xfffc) == 0 {
            tracing::error!("iret_protected: return CS selector null, ESP={:#x} icount={}", temp_esp, self.icount);
            return self.exception(Exception::Gp, 0);
        }

        // Parse CS selector and fetch/validate descriptor from GDT
        let mut cs_selector = BxSelector::default();
        parse_selector(raw_cs_raw, &mut cs_selector);

        let (dword1, dword2) = match self.fetch_raw_descriptor(&cs_selector) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_raw & 0xfffc),
        };
        let mut cs_descriptor = match self.parse_descriptor(dword1, dword2) {
            Ok(v) => v,
            Err(_) => return self.exception(Exception::Gp, raw_cs_raw & 0xfffc),
        };

        // Return CS selector RPL must be >= CPL
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cs_selector.rpl < cpl {
            tracing::error!(
                "iret_protected: return selector RPL ({}) < CPL ({})",
                cs_selector.rpl, cpl
            );
            return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
        }

        // Validate code-segment descriptor
        if let Err(_) = self.check_cs(&cs_descriptor, raw_cs_raw, 0, cs_selector.rpl) {
            return self.exception(Exception::Gp, raw_cs_raw & 0xfffc);
        }

        // Compute EFLAGS changeMask based on OLD CPL (before loading new CS)
        let iopl = ((self.eflags >> 12) & 3) as u8;
        let mut change_mask: u32 =
            0x0000_08D5 | // CF, PF, AF, ZF, SF, OF
            0x0000_0100 | // TF
            0x0000_0400 | // DF
            0x0000_4000 | // NT
            0x0001_0000 | // RF
            0x0004_0000 | // AC
            0x0020_0000;  // ID
        if cpl <= iopl {
            change_mask |= 0x0000_0200; // IF
        }
        if cpl == 0 {
            change_mask |= 0x0000_3000 | 0x0008_0000 | 0x0010_0000; // IOPL | VIF | VIP
        }

        let new_cpl = cs_selector.rpl;
        if new_cpl == cpl {
            // ── Same privilege level ─────────────────────────────────────────
            tracing::debug!(
                "IRET32(PM): same-priv return to CS={:#06x} EIP={:#010x} EFLAGS={:#010x}",
                raw_cs_raw, new_eip, new_eflags
            );

            // Load CS from GDT descriptor (sets CS.base from descriptor, NOT << 4)
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_eip as u64, new_cpl)?;

            // Restore EFLAGS with masked bits only
            self.eflags = (self.eflags & !change_mask) | (new_eflags & change_mask);

            // Advance ESP by 12 (EIP + CS-dword + EFLAGS = 3 × 4 bytes)
            if self.is_stack_32bit() {
                let esp = self.esp();
                self.set_esp(esp.wrapping_add(12));
            } else {
                let sp = self.sp();
                self.set_sp(sp.wrapping_add(12));
            }
        } else {
            // ── Privilege change (returning to outer/less-privileged ring) ────
            tracing::debug!(
                "IRET32(PM): privilege change to CS={:#06x} EIP={:#010x} EFLAGS={:#010x}",
                raw_cs_raw, new_eip, new_eflags
            );

            // Read new ESP and SS from stack at ESP+12 and ESP+16
            let new_esp        = self.stack_read_dword(temp_esp + 12)?;
            let raw_ss_raw     = self.stack_read_dword(temp_esp + 16)? as u16;

            if (raw_ss_raw & 0xfffc) == 0 {
                tracing::error!("iret_protected: SS selector null");
                return self.exception(Exception::Gp, 0);
            }

            let mut ss_selector = BxSelector::default();
            parse_selector(raw_ss_raw, &mut ss_selector);

            if ss_selector.rpl != cs_selector.rpl {
                tracing::error!("iret_protected: SS.rpl != CS.rpl");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }

            let (ss_dw1, ss_dw2) = match self.fetch_raw_descriptor(&ss_selector) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Gp, raw_ss_raw & 0xfffc),
            };
            let mut ss_descriptor = match self.parse_descriptor(ss_dw1, ss_dw2) {
                Ok(v) => v,
                Err(_) => return self.exception(Exception::Gp, raw_ss_raw & 0xfffc),
            };

            // SS must be a writable data segment
            if ss_descriptor.valid == 0
                || !ss_descriptor.segment
                || ss_descriptor.r#type >= 8       // code segment
                || (ss_descriptor.r#type & 2) == 0  // not writable
            {
                tracing::error!("iret_protected: SS not writable data segment");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if ss_descriptor.dpl != cs_selector.rpl {
                tracing::error!("iret_protected: SS.dpl != CS.rpl");
                return self.exception(Exception::Gp, raw_ss_raw & 0xfffc);
            }
            if !ss_descriptor.p {
                tracing::error!("iret_protected: SS not present");
                return self.exception(Exception::Np, raw_ss_raw & 0xfffc);
            }

            // Load CS (sets new CPL = new_cpl)
            self.branch_far(&mut cs_selector, &mut cs_descriptor, new_eip as u64, new_cpl)?;

            // Restore EFLAGS (changeMask was computed from old CPL above)
            self.eflags = (self.eflags & !change_mask) | (new_eflags & change_mask);

            // Load SS and restore ESP
            self.load_ss(&mut ss_selector, &mut ss_descriptor, new_cpl)?;
            if self.is_stack_32bit() {
                self.set_esp(new_esp);
            } else {
                self.set_sp(new_esp as u16);
            }

            // validate_seg_regs(): null out DS/ES/FS/GS if no longer accessible
            // (needed for ring-0→ring-3 transitions to prevent leaking kernel selectors)
            // Linux kernel-to-kernel transitions don't need this, but implement stub:
            // TODO: full validate_seg_regs() if user-mode transitions are needed
        }

        Ok(())
    }

    // =========================================================================
    // Real Mode Interrupt Handler
    // =========================================================================
    
    /// Handle interrupt in real mode using IVT
    pub(super) fn interrupt_real_mode(&mut self, vector: u8) -> super::Result<()> {
        // Save current FLAGS, CS, IP on stack
        let flags = (self.eflags & 0xFFFF) as u16;
        let cs = self.sregs[BxSegregs::Cs as usize].selector.value;
        let ip = self.get_ip();
        
        // Push FLAGS, CS, IP
        self.push_16(flags)?;
        self.push_16(cs)?;
        self.push_16(ip)?;
        
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
        // Log INT 15h calls (memory detection) — AH=88h returns extended memory in AX
        if vector == 0x15 {
            tracing::warn!(
                "INT 15h: AH={:#04x} AX={:#06x} → handler at {:04x}:{:04x}, caller was {:04x}:{:04x}",
                self.ah(), self.ax(), new_cs, new_ip, cs, ip
            );
        }
        Ok(())
    }

    // =========================================================================
    // HLT - Halt instruction
    // =========================================================================
    
    /// HLT - Halt CPU until interrupt
    /// In Bochs: Sets activity_state to ActivityStateHlt and raises async_event
    pub fn hlt(&mut self, _instr: &Instruction) {
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
    pub fn cpuid(&mut self, _instr: &Instruction) {
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
