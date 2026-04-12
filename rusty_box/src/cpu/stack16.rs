//! 16-bit Stack operations for x86 CPU emulation
//!
//! Based on Bochs stack16.cc

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, decoder::Instruction, eflags::EFlags};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 16-bit PUSH instructions
    // Based on Bochs 
    // =========================================================================

    /// PUSH r16 - Push 16-bit register
    /// Based on Bochs stack16.cc PUSH_EwR
    pub fn push_ew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst() as usize;
        let value = self.get_gpr16(dst);
        self.push_16(value)?;
        Ok(())
    }

    /// PUSH m16 - Push 16-bit value from memory
    /// Based on Bochs stack16.cc PUSH_EwM
    pub fn push_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let value = self.v_read_word(seg, eaddr)?;
        self.push_16(value)?;
        Ok(())
    }

    /// PUSH Sw - Push segment register
    /// Based on Bochs stack16.cc PUSH16_Sw
    pub fn push16_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let src = instr.src() as usize;
        let value = self.sregs[src].selector.value;
        self.push_16(value)?;
        Ok(())
    }

    /// PUSH imm16
    /// Based on Bochs stack16.cc PUSH_Iw
    pub fn push_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        let value = instr.iw();
        self.push_16(value)?;
        Ok(())
    }

    /// PUSH sign-extended imm8 (16-bit mode)
    /// Based on Bochs stack16.cc PUSH_Ib
    pub fn push_sib16(&mut self, instr: &Instruction) -> super::Result<()> {
        // Sign-extend 8-bit immediate to 16-bit
        let imm8 = instr.ib() as i8;
        let value = imm8 as i16 as u16;
        self.push_16(value)?;
        Ok(())
    }

    // =========================================================================
    // 16-bit POP instructions
    // Based on Bochs 
    // =========================================================================

    /// POP r16 - Pop into 16-bit register
    /// Based on Bochs stack16.cc POP_EwR
    pub fn pop_ew_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let dst = instr.dst() as usize;
        let value = self.pop_16()?;
        self.set_gpr16(dst, value);
        Ok(())
    }

    /// POP m16 - Pop into 16-bit memory location
    /// Based on Bochs stack16.cc POP_EwM
    pub fn pop_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let value = self.pop_16()?;
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        self.v_write_word(seg, eaddr, value)?;
        Ok(())
    }

    /// POP Sw - Pop into segment register
    /// Based on Bochs stack16.cc POP16_Sw
    pub fn pop16_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let selector_value = self.pop_16()?;
        let seg = super::decoder::BxSegregs::from(instr.dst());

        self.load_seg_reg(seg, selector_value)?;

        // SS interrupt inhibition: Bochs 
        if seg == super::decoder::BxSegregs::Ss {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        }

        Ok(())
    }

    // =========================================================================
    // PUSHA16/POPA16 instructions
    // Based on Bochs 
    // =========================================================================

    /// PUSHA - Push all 16-bit general registers
    /// Push order: AX, CX, DX, BX, SP (original), BP, SI, DI
    /// Based on Bochs 
    pub fn pusha16(&mut self, _instr: &Instruction) -> super::Result<()> {
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
            self.stack_write_word(temp_esp.wrapping_sub(2), ax)?;
            self.stack_write_word(temp_esp.wrapping_sub(4), cx)?;
            self.stack_write_word(temp_esp.wrapping_sub(6), dx)?;
            self.stack_write_word(temp_esp.wrapping_sub(8), bx)?;
            self.stack_write_word(temp_esp.wrapping_sub(10), temp_sp)?;
            self.stack_write_word(temp_esp.wrapping_sub(12), bp)?;
            self.stack_write_word(temp_esp.wrapping_sub(14), si)?;
            self.stack_write_word(temp_esp.wrapping_sub(16), di)?;

            self.set_esp(temp_esp.wrapping_sub(16));
        } else {
            let temp_sp = self.sp();

            // Write all registers to stack at their final positions
            self.stack_write_word(temp_sp.wrapping_sub(2) as u32, ax)?;
            self.stack_write_word(temp_sp.wrapping_sub(4) as u32, cx)?;
            self.stack_write_word(temp_sp.wrapping_sub(6) as u32, dx)?;
            self.stack_write_word(temp_sp.wrapping_sub(8) as u32, bx)?;
            self.stack_write_word(temp_sp.wrapping_sub(10) as u32, temp_sp)?;
            self.stack_write_word(temp_sp.wrapping_sub(12) as u32, bp)?;
            self.stack_write_word(temp_sp.wrapping_sub(14) as u32, si)?;
            self.stack_write_word(temp_sp.wrapping_sub(16) as u32, di)?;

            self.set_sp(temp_sp.wrapping_sub(16));
        }

        Ok(())
    }

    /// POPA - Pop all 16-bit general registers
    /// Pop order: DI, SI, BP, (skip SP), BX, DX, CX, AX
    /// Based on Bochs 
    pub fn popa16(&mut self, _instr: &Instruction) -> super::Result<()> {
        let (di, si, bp, bx, dx, cx, ax) = if self.is_stack_32bit() {
            let temp_esp = self.esp();

            let di = self.stack_read_word(temp_esp)?;
            let si = self.stack_read_word(temp_esp.wrapping_add(2))?;
            let bp = self.stack_read_word(temp_esp.wrapping_add(4))?;
            // Skip reading SP at offset +6 (it's discarded)
            let _sp_skip = self.stack_read_word(temp_esp.wrapping_add(6))?;
            let bx = self.stack_read_word(temp_esp.wrapping_add(8))?;
            let dx = self.stack_read_word(temp_esp.wrapping_add(10))?;
            let cx = self.stack_read_word(temp_esp.wrapping_add(12))?;
            let ax = self.stack_read_word(temp_esp.wrapping_add(14))?;

            self.set_esp(temp_esp.wrapping_add(16));

            (di, si, bp, bx, dx, cx, ax)
        } else {
            let temp_sp = self.sp();

            let di = self.stack_read_word(temp_sp as u32)?;
            let si = self.stack_read_word(temp_sp.wrapping_add(2) as u32)?;
            let bp = self.stack_read_word(temp_sp.wrapping_add(4) as u32)?;
            // Skip reading SP at offset +6 (it's discarded)
            let _sp_skip = self.stack_read_word(temp_sp.wrapping_add(6) as u32)?;
            let bx = self.stack_read_word(temp_sp.wrapping_add(8) as u32)?;
            let dx = self.stack_read_word(temp_sp.wrapping_add(10) as u32)?;
            let cx = self.stack_read_word(temp_sp.wrapping_add(12) as u32)?;
            let ax = self.stack_read_word(temp_sp.wrapping_add(14) as u32)?;

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

        Ok(())
    }

    // =========================================================================
    // PUSHF/POPF instructions (16-bit)
    // Based on Bochs flag_ctrl.cc (but traditionally in stack16.cc)
    // =========================================================================

    /// PUSHF - Push flags (16-bit)
    /// Based on Bochs  PUSHF_Fw
    pub fn pushf_fw(&mut self, _instr: &Instruction) -> super::Result<()> {
        let mut flags = (self.eflags.bits() & 0xFFFF) as u16;

        if self.v8086_mode()
            && self.eflags.iopl() < 3 {
                if self.cr4.vme() {
                    // VME: push IOPL=3, replace IF with VIF
                    flags |= EFlags::IOPL_MASK.bits() as u16;
                    if self.eflags.contains(EFlags::VIF) {
                        flags |= EFlags::IF_.bits() as u16;
                    } else {
                        flags &= !(EFlags::IF_.bits() as u16);
                    }
                } else {
                    tracing::debug!("PUSHFW: #GP(0) in v8086 (no VME) mode");
                    self.exception(super::cpu::Exception::Gp, 0)?;
                }
            }

        self.push_16(flags)?;
        Ok(())
    }

    /// POPF - Pop flags (16-bit)
    /// Based on Bochs  POPF_Fw
    pub fn popf_fw(&mut self, _instr: &Instruction) -> super::Result<()> {
        use super::decoder::BxSegregs;

        // Base changeMask: OSZAPC + TF + DF + NT
        let mut change_mask: u32 =
            EFlags::OSZAPC.bits() | EFlags::TF.bits() | EFlags::DF.bits() | EFlags::NT.bits();

        // RSP_SPECULATIVE (conceptual - we'll adjust ESP after)
        let flags16 = self.pop_16()?;

        if self.protected_mode() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl as u32;
            if cpl == 0 {
                change_mask |= EFlags::IOPL_MASK.bits();
            }
            if cpl <= self.eflags.iopl() as u32 {
                change_mask |= EFlags::IF_.bits();
            }
        } else if self.v8086_mode() {
            if self.eflags.iopl() < 3 {
                if self.cr4.vme() {
                    // VME path
                    if ((flags16 as u32 & EFlags::IF_.bits()) != 0
                        && self.eflags.contains(EFlags::VIP))
                        || (flags16 as u32 & EFlags::TF.bits()) != 0
                    {
                        tracing::debug!("POPFW: #GP(0) in VME mode");
                        self.exception(super::cpu::Exception::Gp, 0)?;
                    }
                    // IF, IOPL unchanged; VIF = flags16.IF
                    change_mask |= EFlags::VIF.bits();
                    let mut flags32 = flags16 as u32;
                    if flags32 & EFlags::IF_.bits() != 0 {
                        flags32 |= EFlags::VIF.bits();
                    }
                    self.write_eflags(flags32, change_mask);
                    return Ok(());
                }
                tracing::debug!("POPFW: #GP(0) in v8086 (no VME) mode");
                self.exception(super::cpu::Exception::Gp, 0)?;
            }
            change_mask |= EFlags::IF_.bits();
        } else {
            // Real mode: all non-reserved flags can be modified
            change_mask |= EFlags::IOPL_MASK.bits() | EFlags::IF_.bits();
        }

        self.write_eflags(flags16 as u32, change_mask);
        Ok(())
    }

    // =========================================================================
    // PUSH/POP Sw - 16-bit mode segment register push/pop (unified dispatch)
    // =========================================================================

    /// PUSH Sw (16-bit opsize) - Push segment register
    /// Used by the PushOp16Sw opcode. Bochs: i->src() for PUSH Sw
    pub fn push_op16_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let seg = instr.src() as usize;
        let val = self.sregs[seg].selector.value;
        self.push_16(val)?;
        Ok(())
    }

    /// POP Sw (16-bit opsize) - Pop into segment register from operands.dst
    /// Used by the PopOp16Sw opcode
    pub fn pop_op16_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let selector_value = self.pop_16()?;
        let seg = super::decoder::BxSegregs::from(instr.dst());

        self.load_seg_reg(seg, selector_value)?;

        if seg == super::decoder::BxSegregs::Ss {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        }

        Ok(())
    }

    // =========================================================================
    // Unified PUSH/POP Ew dispatch (register vs memory)
    // =========================================================================

    /// PUSH r/m16 - Unified dispatch based on mod_c0()
    pub fn push_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.push_ew_r(instr)
        } else {
            self.push_ew_m(instr)
        }
    }

    /// POP r/m16 - Unified dispatch based on mod_c0()
    pub fn pop_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.pop_ew_r(instr)
        } else {
            self.pop_ew_m(instr)
        }
    }

    // =========================================================================
    // ENTER instruction (16-bit)
    // Based on Bochs  ENTER16_IwIb
    // =========================================================================

    /// ENTER (16-bit operand size)
    /// Based on Bochs  ENTER16_IwIb
    pub fn enter16_iw_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm16 = instr.iw();
        let mut level = instr.ib2() & 0x1F;

        self.push_16(self.bp())?;
        let frame_ptr16 = self.sp();

        if self.is_stack_32bit() {
            let mut ebp = self.ebp(); // Use temp copy for case of exception

            if level > 0 {
                // do level-1 times
                while {
                    level -= 1;
                    level
                } > 0
                {
                    ebp = ebp.wrapping_sub(2);
                    let temp16 = self.stack_read_word(ebp)?;
                    self.push_16(temp16)?;
                }

                // push(frame pointer)
                self.push_16(frame_ptr16)?;
            }

            self.set_esp(self.esp().wrapping_sub(imm16 as u32));

            // ENTER finishes with memory write check on the final stack pointer
            // the memory is touched but no write actually occurs
            // emulate it by doing RMW read access from SS:ESP
            let esp = self.esp();
            self.v_read_rmw_word(super::decoder::BxSegregs::Ss, esp)?;

            self.set_bp(frame_ptr16);
        } else {
            let mut bp = self.bp() as u32;

            if level > 0 {
                // do level-1 times
                while {
                    level -= 1;
                    level
                } > 0
                {
                    bp = bp.wrapping_sub(2) & 0xFFFF;
                    let temp16 = self.stack_read_word(bp)?;
                    self.push_16(temp16)?;
                }

                // push(frame pointer)
                self.push_16(frame_ptr16)?;
            }

            self.set_sp(self.sp().wrapping_sub(imm16));

            // ENTER finishes with memory write check on the final stack pointer
            // the memory is touched but no write actually occurs
            // emulate it by doing RMW read access from SS:SP
            let sp = self.sp() as u32;
            self.v_read_rmw_word(super::decoder::BxSegregs::Ss, sp)?;
        }

        self.set_bp(frame_ptr16);
        Ok(())
    }

    // =========================================================================
    // LEAVE instruction
    // =========================================================================

    /// LEAVE - High level procedure exit (16-bit)
    /// Bochs 
    pub fn leave16(&mut self, _instr: &Instruction) -> super::Result<()> {
        // Bochs : check SS.D/B for 32-bit vs 16-bit stack
        let value16;
        if self.is_stack_32bit() {
            // 32-bit stack mode: use full EBP address, set full ESP
            let ebp = self.ebp();
            value16 = self.stack_read_word(ebp)?;
            self.set_esp(ebp.wrapping_add(2));
        } else {
            // 16-bit stack mode: use BP address, set SP
            let bp = self.bp();
            value16 = self.stack_read_word(bp as u32)?;
            self.set_sp(bp.wrapping_add(2));
        }
        self.set_bp(value16);
        Ok(())
    }
}
