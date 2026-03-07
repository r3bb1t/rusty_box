//! 32-bit Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack32.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, decoder::Instruction, eflags::EFlags};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 32-bit PUSH instructions
    // Based on Bochs stack32.cc:27-70
    // =========================================================================

    /// PUSH r32 - Push 32-bit register
    /// Based on Bochs stack32.cc PUSH_EdR
    pub fn push_ed_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst() as usize;
        let value = self.get_gpr32(dst);
        self.push_32(value)?;
        tracing::trace!("PUSH r32 (reg {}): {:#010x}", dst, value);
        Ok(())
    }

    /// PUSH m32 - Push 32-bit value from memory
    /// Based on Bochs stack32.cc PUSH_EdM
    pub fn push_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let value = self.read_virtual_dword(seg, eaddr)?;
        self.push_32(value)?;
        tracing::trace!("PUSH m32 [{:?}:{:#010x}]: {:#010x}", seg, eaddr, value);
        Ok(())
    }

    /// PUSH imm32
    /// Based on Bochs stack32.cc PUSH_Id
    pub fn push_id(&mut self, instr: &Instruction) -> super::Result<()> {
        let value = instr.id();
        self.push_32(value)?;
        tracing::trace!("PUSH imm32: {:#010x}", value);
        Ok(())
    }

    // =========================================================================
    // 32-bit POP instructions
    // Based on Bochs stack32.cc:72-118
    // =========================================================================

    /// POP r32 - Pop into 32-bit register
    /// Based on Bochs stack32.cc POP_EdR
    pub fn pop_ed_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst() as usize;
        let value = self.pop_32()?;
        self.set_gpr32(dst, value);
        tracing::trace!("POP r32 (reg {}): {:#010x}", dst, value);
        Ok(())
    }

    /// POP m32 - Pop into 32-bit memory location
    /// Based on Bochs stack32.cc POP_EdM
    pub fn pop_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let value = self.pop_32()?;
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        self.write_virtual_dword(seg, eaddr, value)?;
        tracing::trace!("POP m32 [{:?}:{:#010x}]: {:#010x}", seg, eaddr, value);
        Ok(())
    }

    /// POP segment register (32-bit mode)
    /// Based on Bochs stack32.cc:87-111 POP32_Sw
    /// Pops a 16-bit selector from stack (advancing ESP by 4) and loads it into segment register
    pub fn pop32_sw(&mut self, instr: &Instruction) -> Result<(), super::error::CpuError> {
        use crate::cpu::decoder::BxSegregs;

        // Bochs POP32_Sw: pop 32-bit value, use low 16 bits as selector
        let val32 = self.pop_32()?;
        let selector_value = val32 as u16;
        let seg = BxSegregs::from(instr.dst());

        self.load_seg_reg(seg, selector_value)?;

        // POP SS inhibits interrupts until next instruction boundary
        // (Bochs stack32.cc:102-108)
        if seg == BxSegregs::Ss {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        }

        Ok(())
    }

    // =========================================================================
    // Unified PUSH/POP Ed dispatch (register vs memory)
    // =========================================================================

    /// PUSH r/m32 - Unified dispatch based on mod_c0()
    pub fn push_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.push_ed_r(instr)
        } else {
            self.push_ed_m(instr)
        }
    }

    /// POP r/m32 - Unified dispatch based on mod_c0()
    pub fn pop_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.pop_ed_r(instr)
        } else {
            self.pop_ed_m(instr)
        }
    }

    // =========================================================================
    // PUSHAD/POPAD instructions
    // Based on Bochs stack32.cc:120-193
    // =========================================================================

    /// PUSHAD - Push all 32-bit general registers
    /// Push order: EAX, ECX, EDX, EBX, ESP (original), EBP, ESI, EDI
    /// Based on Bochs stack32.cc:120-151
    pub fn pusha32(&mut self, _instr: &Instruction) -> super::Result<()> {
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
            self.stack_write_dword(temp_esp.wrapping_sub(4), eax)?;
            self.stack_write_dword(temp_esp.wrapping_sub(8), ecx)?;
            self.stack_write_dword(temp_esp.wrapping_sub(12), edx)?;
            self.stack_write_dword(temp_esp.wrapping_sub(16), ebx)?;
            self.stack_write_dword(temp_esp.wrapping_sub(20), temp_esp)?;
            self.stack_write_dword(temp_esp.wrapping_sub(24), ebp)?;
            self.stack_write_dword(temp_esp.wrapping_sub(28), esi)?;
            self.stack_write_dword(temp_esp.wrapping_sub(32), edi)?;

            self.set_esp(temp_esp.wrapping_sub(32));
        } else {
            let temp_sp = self.sp();
            let temp_esp = self.esp();

            // Write all registers to stack at their final positions
            self.stack_write_dword(temp_sp.wrapping_sub(4) as u32, eax)?;
            self.stack_write_dword(temp_sp.wrapping_sub(8) as u32, ecx)?;
            self.stack_write_dword(temp_sp.wrapping_sub(12) as u32, edx)?;
            self.stack_write_dword(temp_sp.wrapping_sub(16) as u32, ebx)?;
            self.stack_write_dword(temp_sp.wrapping_sub(20) as u32, temp_esp)?;
            self.stack_write_dword(temp_sp.wrapping_sub(24) as u32, ebp)?;
            self.stack_write_dword(temp_sp.wrapping_sub(28) as u32, esi)?;
            self.stack_write_dword(temp_sp.wrapping_sub(32) as u32, edi)?;

            self.set_sp(temp_sp.wrapping_sub(32));
        }

        tracing::trace!(
            "PUSHAD: EAX={:08x} ECX={:08x} EDX={:08x} EBX={:08x} EBP={:08x} ESI={:08x} EDI={:08x}",
            eax,
            ecx,
            edx,
            ebx,
            ebp,
            esi,
            edi
        );
        Ok(())
    }

    /// POPAD - Pop all 32-bit general registers
    /// Pop order: EDI, ESI, EBP, (skip ESP), EBX, EDX, ECX, EAX
    /// Based on Bochs stack32.cc:153-193
    pub fn popa32(&mut self, _instr: &Instruction) -> super::Result<()> {
        let (edi, esi, ebp, ebx, edx, ecx, eax) = if self.is_stack_32bit() {
            let temp_esp = self.esp();

            let edi = self.stack_read_dword(temp_esp)?;
            let esi = self.stack_read_dword(temp_esp.wrapping_add(4))?;
            let ebp = self.stack_read_dword(temp_esp.wrapping_add(8))?;
            // Skip reading ESP at offset +12 (it's discarded)
            let _esp_skip = self.stack_read_dword(temp_esp.wrapping_add(12))?;
            let ebx = self.stack_read_dword(temp_esp.wrapping_add(16))?;
            let edx = self.stack_read_dword(temp_esp.wrapping_add(20))?;
            let ecx = self.stack_read_dword(temp_esp.wrapping_add(24))?;
            let eax = self.stack_read_dword(temp_esp.wrapping_add(28))?;

            self.set_esp(temp_esp.wrapping_add(32));

            (edi, esi, ebp, ebx, edx, ecx, eax)
        } else {
            let temp_sp = self.sp();

            let edi = self.stack_read_dword(temp_sp as u32)?;
            let esi = self.stack_read_dword(temp_sp.wrapping_add(4) as u32)?;
            let ebp = self.stack_read_dword(temp_sp.wrapping_add(8) as u32)?;
            // Skip reading ESP at offset +12 (it's discarded)
            let _esp_skip = self.stack_read_dword(temp_sp.wrapping_add(12) as u32)?;
            let ebx = self.stack_read_dword(temp_sp.wrapping_add(16) as u32)?;
            let edx = self.stack_read_dword(temp_sp.wrapping_add(20) as u32)?;
            let ecx = self.stack_read_dword(temp_sp.wrapping_add(24) as u32)?;
            let eax = self.stack_read_dword(temp_sp.wrapping_add(28) as u32)?;

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

        tracing::trace!(
            "POPAD: EDI={:08x} ESI={:08x} EBP={:08x} EBX={:08x} EDX={:08x} ECX={:08x} EAX={:08x}",
            edi,
            esi,
            ebp,
            ebx,
            edx,
            ecx,
            eax
        );
        Ok(())
    }

    // =========================================================================
    // PUSHFD/POPFD instructions (32-bit)
    // Based on Bochs flag_ctrl.cc (but traditionally in stack32.cc)
    // =========================================================================

    /// PUSHFD - Push flags (32-bit)
    /// Based on Bochs flag_ctrl.cc:273-290 PUSHF_Fd
    pub fn pushf_fd(&mut self, _instr: &Instruction) -> super::Result<()> {
        if self.v8086_mode() && self.eflags.iopl() < 3 {
            tracing::debug!("PUSHFD: #GP(0) in v8086 mode");
            self.exception(super::cpu::Exception::Gp, 0)?;
        }

        // VM & RF flags cleared in image stored on the stack
        let flags = self.eflags.bits() & 0x00FCFFFF;
        self.push_32(flags)?;
        Ok(())
    }

    /// POPFD - Pop flags (32-bit)
    /// Based on Bochs flag_ctrl.cc:292-340 POPF_Fd
    pub fn popf_fd(&mut self, _instr: &Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        // Base changeMask: OSZAPC + TF + DF + NT + RF + ID + AC
        let mut change_mask: u32 = EFlags::OSZAPC.bits()
            | EFlags::TF.bits()
            | EFlags::DF.bits()
            | EFlags::NT.bits()
            | EFlags::RF.bits()
            | EFlags::ID.bits()
            | EFlags::AC.bits();

        // RF is always zero after the execution of POPF
        let flags32 = self.pop_32()? & !EFlags::RF.bits();

        if self.protected_mode() {
            // IOPL changed only if CPL == 0
            // IF changed only if CPL <= EFLAGS.IOPL
            // VIF, VIP, VM are unaffected
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl as u32;
            if cpl == 0 {
                change_mask |= EFlags::IOPL_MASK.bits();
            }
            if cpl <= self.eflags.iopl() as u32 {
                change_mask |= EFlags::IF_.bits();
            }
        } else if self.v8086_mode() {
            if self.eflags.iopl() < 3 {
                tracing::debug!("POPFD: #GP(0) in v8086 mode");
                self.exception(super::cpu::Exception::Gp, 0)?;
            }
            // v8086-mode: VM, IOPL, VIP, VIF are unaffected
            change_mask |= EFlags::IF_.bits();
        } else {
            // Real mode: VIF, VIP, VM are unaffected
            change_mask |= EFlags::IOPL_MASK.bits() | EFlags::IF_.bits();
        }

        self.write_eflags(flags32, change_mask);
        Ok(())
    }

    /// PUSH segment register (32-bit mode)
    /// Based on Bochs stack32.cc:70-85 PUSH32_Sw
    /// Pushes 4 bytes (only lower 16 bits are meaningful)
    pub fn push_op32_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let seg_idx = instr.dst() as usize; // nnn field = segment register index
        let val_16 = self.sregs[seg_idx].selector.value;
        // Bochs writes only a word at ESP-4, not a full dword
        let ss_d_b = unsafe {
            self.sregs[super::decoder::BxSegregs::Ss as usize]
                .cache
                .u
                .segment
                .d_b
        };
        if ss_d_b {
            let esp = self.get_gpr32(4);
            self.stack_write_word(esp.wrapping_sub(4), val_16)?;
            self.set_gpr32(4, esp.wrapping_sub(4));
        } else {
            let sp = self.get_gpr16(4);
            self.stack_write_word(sp.wrapping_sub(4) as u32, val_16)?;
            self.set_gpr16(4, sp.wrapping_sub(4));
        }
        Ok(())
    }

    /// ENTER (32-bit operand size)
    /// Based on Bochs stack32.cc:195-256 ENTER32_IwIb
    pub fn enter32_iw_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm16 = instr.iw() as u32;
        let mut level = instr.ib2() & 0x1F;

        self.push_32(self.ebp())?;
        let frame_ptr32 = self.esp();

        if self.is_stack_32bit() {
            let mut ebp = self.ebp(); // Use temp copy for case of exception

            if level > 0 {
                // do level-1 times
                while {
                    level -= 1;
                    level
                } > 0
                {
                    ebp = ebp.wrapping_sub(4);
                    let temp32 = self.stack_read_dword(ebp)?;
                    self.push_32(temp32)?;
                }

                // push(frame pointer)
                self.push_32(frame_ptr32)?;
            }

            self.set_esp(self.esp().wrapping_sub(imm16));

            // ENTER finishes with memory write check on the final stack pointer
            // the memory is touched but no write actually occurs
            // emulate it by doing RMW read access from SS:ESP
            let esp = self.esp();
            self.read_rmw_virtual_dword(super::decoder::BxSegregs::Ss, esp)?;
        } else {
            let mut bp = self.bp() as u32;

            if level > 0 {
                // do level-1 times
                while {
                    level -= 1;
                    level
                } > 0
                {
                    bp = bp.wrapping_sub(4) & 0xFFFF;
                    let temp32 = self.stack_read_dword(bp)?;
                    self.push_32(temp32)?;
                }

                // push(frame pointer)
                self.push_32(frame_ptr32)?;
            }

            self.set_sp(self.sp().wrapping_sub(imm16 as u16));

            // ENTER finishes with memory write check on the final stack pointer
            // the memory is touched but no write actually occurs
            // emulate it by doing RMW read access from SS:SP
            let sp = self.sp() as u32;
            self.read_rmw_virtual_dword(super::decoder::BxSegregs::Ss, sp)?;
        }

        self.set_ebp(frame_ptr32);
        Ok(())
    }

    /// LEAVE (32-bit operand size)
    /// Based on Bochs stack32.cc:258-273
    pub fn leave_op32(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        let ss_d_b = unsafe {
            self.sregs[super::decoder::BxSegregs::Ss as usize]
                .cache
                .u
                .segment
                .d_b
        };
        let value32 = if ss_d_b {
            // 32-bit stack
            let ebp = self.get_gpr32(5); // EBP
            let val = self.stack_read_dword(ebp)?;
            self.set_gpr32(4, ebp.wrapping_add(4)); // ESP = EBP + 4
            val
        } else {
            // 16-bit stack
            let bp = self.get_gpr16(5) as u32; // BP
            let val = self.stack_read_dword(bp)?;
            self.set_gpr16(4, bp.wrapping_add(4) as u16); // SP = BP + 4
            val
        };
        self.set_gpr32(5, value32); // EBP = [old EBP]
        Ok(())
    }
}
