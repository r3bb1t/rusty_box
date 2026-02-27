//! 32-bit Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack32.cc
//! Copyright (C) 2001-2018 The Bochs Project

use alloc::vec::Vec;

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    eflags::EFlags,
};

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
        use crate::cpu::segment_ctrl_pro::parse_selector;

        // Pop 16-bit selector from stack
        // In 32-bit mode, ESP advances by 4 even though only 2 bytes are used
        let selector_value = self.stack_read_word(self.esp())? as u16;

        // Get destination segment register from instruction
        let seg_idx = instr.dst() as usize;
        let seg = BxSegregs::from(seg_idx as u8);

        // Load segment register
        // Original Bochs: load_seg_reg(&BX_CPU_THIS_PTR sregs[i->dst()], selector);
        let in_real_mode = self.real_mode();

        if in_real_mode {
            // Real mode: simple base = selector << 4
            self.load_seg_reg_real_mode(seg, selector_value);
        } else {
            // Protected mode: check for NULL selector first
            // Based on Bochs segment_ctrl_pro.cc:40,108 - check (new_value & 0xfffc) == 0
            let is_null_selector = (selector_value & 0xfffc) == 0;

            if is_null_selector {
                // NULL selector handling
                if seg_idx == BxSegregs::Ss as usize {
                    // SS cannot be NULL in protected mode (except 64-bit mode with special conditions)
                    // Bochs segment_ctrl_pro.cc:48-49
                    tracing::debug!("POP SS: loading NULL selector in protected mode - #GP");
                    self.exception(super::cpu::Exception::Gp, 0)?;
                    return Ok(());
                } else {
                    // DS/ES/FS/GS can be NULL - just invalidate the segment
                    // Based on Bochs load_null_selector() in segment_ctrl_pro.cc:212-234
                    tracing::debug!("POP seg{}: loading NULL selector (allowed)", seg_idx);
                    self.load_null_selector(seg, selector_value);
                }
            } else {
                // Non-NULL selector: fetch descriptor and load
                let mut selector = super::descriptor::BxSelector::default();
                parse_selector(selector_value, &mut selector);

                let fetch_result = self.fetch_raw_descriptor(&selector);
                if fetch_result.is_err() {
                    let ss_base = unsafe { self.sregs[BxSegregs::Ss as usize].cache.u.segment.base };
                    let laddr = ss_base + self.esp() as u64;
                    // Try to translate and show what's at the stack
                    let paddr = self.translate_data_read(laddr).unwrap_or(0xDEAD);
                    tracing::warn!("POP32_Sw: fetch_raw_descriptor FAILED for selector={:#06x} (index={}, TI={}), seg_idx={}, icount={}",
                        selector_value, selector.index, selector.ti, seg_idx, self.icount);
                    tracing::warn!("  ESP={:#x} SS.base={:#x} laddr={:#x} paddr={:#x}", self.esp(), ss_base, laddr, paddr);
                    tracing::warn!("  GDTR.base={:#x} GDTR.limit={:#x} CR3={:#x}", self.gdtr.base, self.gdtr.limit, self.cr3);
                    // Dump stack dwords around ESP
                    let esp = self.esp();
                    for i in 0..8u32 {
                        let offset = esp.wrapping_add(i * 4);
                        let la = ss_base + offset as u64;
                        if let Ok(pa) = self.translate_data_read(la) {
                            let val = self.mem_read_dword(pa);
                            tracing::warn!("  Stack[ESP+{:#x}] = {:#010x}  (laddr={:#x} paddr={:#x})", i*4, val, la, pa);
                        }
                    }
                    // Also show previous ESP entries (what was popped before)
                    for i in 1..5u32 {
                        let offset = esp.wrapping_sub(i * 4);
                        let la = ss_base + offset as u64;
                        if let Ok(pa) = self.translate_data_read(la) {
                            let val = self.mem_read_dword(pa);
                            tracing::warn!("  Stack[ESP-{:#x}] = {:#010x}  (laddr={:#x} paddr={:#x})", i*4, val, la, pa);
                        }
                    }
                    // Dump code bytes near the failing instruction
                    let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                    let code_vaddr = cs_base + self.eip() as u64;
                    // Read 32 bytes before and 16 bytes after
                    let mut code_bytes = Vec::new();
                    for i in 0..48u64 {
                        let va = code_vaddr.wrapping_sub(32).wrapping_add(i);
                        if let Ok(pa) = self.translate_data_read(va) {
                            code_bytes.push(self.mem_read_byte(pa));
                        } else {
                            code_bytes.push(0xCC);
                        }
                    }
                    tracing::warn!("  Code at CS:EIP-32 to CS:EIP+16 (EIP={:#x}, CS.base={:#x}):", self.eip(), cs_base);
                    tracing::warn!("  {:02x?}", &code_bytes);
                }
                let (dword1, dword2) = fetch_result?;
                let mut descriptor = self.parse_descriptor(dword1, dword2)?;

                if seg_idx == BxSegregs::Ss as usize {
                    // Load SS with proper checks and D/B bit
                    // CPL = Current Privilege Level = CS.selector.rpl
                    let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;

                    self.load_ss(&mut selector, &mut descriptor, cpl)?;
                } else {
                    // For other segments, just copy the descriptor
                    // TODO: Implement full load_seg_reg for DS, ES, FS, GS
                    self.sregs[seg as usize].selector = selector;
                    self.sregs[seg as usize].cache = descriptor;
                    self.sregs[seg as usize].cache.valid = super::descriptor::SEG_VALID_CACHE;
                }
            }
        }

        // Advance ESP by 4 (32-bit operand size, even though selector is 16-bit)
        self.set_esp(self.esp().wrapping_add(4));

        // POP SS inhibits interrupts until next instruction boundary
        // (Bochs stack32.cc:102-108)
        if seg_idx == BxSegregs::Ss as usize {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        }

        tracing::trace!("POP seg{}: selector={:#06x}", seg_idx, selector_value);
        Ok(())
    }

    // =========================================================================
    // Unified PUSH/POP Ed dispatch (register vs memory)
    // =========================================================================

    /// PUSH r/m32 - Unified dispatch based on mod_c0()
    pub fn push_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() { self.push_ed_r(instr) } else { self.push_ed_m(instr) }
    }

    /// POP r/m32 - Unified dispatch based on mod_c0()
    pub fn pop_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() { self.pop_ed_r(instr) } else { self.pop_ed_m(instr) }
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

        tracing::trace!("PUSHAD: EAX={:08x} ECX={:08x} EDX={:08x} EBX={:08x} EBP={:08x} ESI={:08x} EDI={:08x}",
            eax, ecx, edx, ebx, ebp, esi, edi);
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
            let _ = self.stack_read_dword(temp_esp.wrapping_add(12))?;
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
            let _ = self.stack_read_dword(temp_sp.wrapping_add(12) as u32)?;
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

        tracing::trace!("POPAD: EDI={:08x} ESI={:08x} EBP={:08x} EBX={:08x} EDX={:08x} ECX={:08x} EAX={:08x}",
            edi, esi, ebp, ebx, edx, ecx, eax);
        Ok(())
    }

    // =========================================================================
    // PUSHFD/POPFD instructions (32-bit)
    // Based on Bochs flag_ctrl.cc (but traditionally in stack32.cc)
    // =========================================================================

    /// PUSHFD - Push flags (32-bit)
    pub fn pushf_fd(&mut self, _instr: &Instruction) -> super::Result<()> {
        // VM & RF flags cleared in image stored on the stack
        let flags = self.eflags.bits() & 0x00FCFFFF;
        self.push_32(flags)?;
        tracing::trace!("PUSHFD: {:#010x}", flags);
        Ok(())
    }

    /// POPFD - Pop flags (32-bit)
    pub fn popf_fd(&mut self, _instr: &Instruction) -> super::Result<()> {
        let flags = self.pop_32()?;

        // RF is always zero after POPF
        // VM, VIP, VIF are unaffected in protected mode
        const CHANGE_MASK: u32 = 0x00244FD5;

        self.eflags = EFlags::from_bits_retain((self.eflags.bits() & !CHANGE_MASK) | (flags & CHANGE_MASK));
        tracing::trace!("POPFD: {:#010x}", flags);
        Ok(())
    }

    /// PUSH segment register (32-bit mode)
    /// Based on Bochs stack32.cc:70-85 PUSH32_Sw
    /// Pushes 4 bytes (only lower 16 bits are meaningful)
    pub fn push_op32_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let seg_idx = instr.dst() as usize; // nnn field = segment register index
        let val_16 = self.sregs[seg_idx].selector.value;
        // Bochs writes only a word at ESP-4, not a full dword
        let ss_d_b = unsafe { self.sregs[super::decoder::BxSegregs::Ss as usize].cache.u.segment.d_b };
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

    /// LEAVE (32-bit operand size)
    /// Based on Bochs stack32.cc:258-273
    pub fn leave_op32(&mut self, _instr: &super::decoder::Instruction) -> super::Result<()> {
        let ss_d_b = unsafe { self.sregs[super::decoder::BxSegregs::Ss as usize].cache.u.segment.d_b };
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
