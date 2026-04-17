//! 64-bit data transfer instructions for x86 CPU emulation
//!
//! Based on Bochs data_xfer64.cc

use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::{BxCpuC, BxCpuIdTrait, Result};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // 64-bit MOV instructions
    // =========================================================================

    /// MOV r64, imm64 (register form)
    /// Matching C++ data_xfer64.cc MOV_RRXIq
    pub fn mov_rrxiq(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let imm64 = instr.iq();

        self.set_gpr64(dst, imm64);
    }

    /// MOV r32, r/m32 (memory form, 64-bit addressing)
    /// Matching C++ data_xfer64.cc MOV64_GdEdM
    pub fn mov64_gd_ed_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let val32 = self.read_virtual_dword_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, val32);
        Ok(())
    }

    /// MOV r32, r32 (register form, 64-bit mode)
    /// Matching C++ data_xfer64.cc MOV64_EdGdR — zero-extends to 64 bits
    pub fn mov64_ed_gd_r(&mut self, instr: &Instruction) {
        let val32 = self.get_gpr32(instr.src() as usize);
        self.set_gpr32(instr.dst() as usize, val32);
    }

    /// MOV r32, r/m32 (register form, 64-bit addressing)
    /// Matching C++ data_xfer64.cc MOV64_GdEdR — zero-extends to 64 bits
    pub fn mov64_gd_ed_r(&mut self, instr: &Instruction) {
        let val32 = self.get_gpr32(instr.src() as usize);
        self.set_gpr32(instr.dst() as usize, val32);
    }

    /// MOV r/m32, r32 (memory form, 64-bit addressing)
    /// Matching C++ data_xfer64.cc MOV64_EdGdM
    pub fn mov64_ed_gd_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let src_reg = instr.src() as usize;
        let val32 = self.get_gpr32(src_reg);

        self.write_virtual_dword_64(seg, eaddr, val32)?;
        Ok(())
    }

    /// MOV r/m64, r64 (memory form)
    /// Matching C++ data_xfer64.cc MOV_EqGqM
    pub fn mov_eq_gq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let src_reg = instr.src() as usize;
        let val64 = self.get_gpr64(src_reg);

        self.write_virtual_qword_64(seg, eaddr, val64)?;
        Ok(())
    }

    /// MOV r/m64, r64 (stack form)
    /// Matching C++ data_xfer64.cc MOV64S_EqGqM
    pub fn mov64s_eq_gq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let src_reg = instr.src() as usize;
        let val64 = self.get_gpr64(src_reg);

        self.stack_write_qword_64(eaddr, val64)?;
        Ok(())
    }

    /// MOV r64, r/m64 (memory form)
    /// Matching C++ data_xfer64.cc MOV_GqEqM
    pub fn mov_gq_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let val64 = self.read_virtual_qword_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        self.set_gpr64(dst_reg, val64);
        Ok(())
    }

    /// MOV r64, r/m64 (stack form)
    /// Matching C++ data_xfer64.cc MOV64S_GqEqM
    pub fn mov64s_gq_eq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let val64 = self.stack_read_qword_64(eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, val64);
        Ok(())
    }

    /// MOV r64, r64 (register form)
    /// Matching C++ data_xfer64.cc MOV_GqEqR
    pub fn mov_gq_eq_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let val64 = self.get_gpr64(src_reg);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, val64);
    }

    /// LEA r64, m - Load effective address into 64-bit register
    /// Matching C++ data_xfer64.cc LEA_GqM
    pub fn lea_gq_m(&mut self, instr: &Instruction) {
        // Bochs: BX_CPU_RESOLVE_ADDR_64(i) = as64L() ? BxResolve64 : BxResolve32
        let eaddr = if instr.as64_l() != 0 {
            self.resolve_addr64(instr)
        } else {
            u64::from(self.resolve_addr32(instr))
        };
        self.set_gpr64(instr.dst() as usize, eaddr);
    }

    /// MOV AL, moffs64
    /// Matching C++ data_xfer64.cc MOV_ALOq
    pub fn mov_aloq(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val8 = self.read_virtual_byte_64(seg, instr.iq())?;

        self.set_gpr8(0, val8); // AL
        Ok(())
    }

    /// MOV moffs64, AL
    /// Matching C++ data_xfer64.cc MOV_OqAL
    pub fn mov_oq_al(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val8 = self.get_gpr8(0); // AL

        self.write_virtual_byte_64(seg, instr.iq(), val8)?;
        Ok(())
    }

    /// MOV AX, moffs64
    /// Matching C++ data_xfer64.cc MOV_AXOq
    pub fn mov_ax_oq(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val16 = self.read_virtual_word_64(seg, instr.iq())?;

        self.set_gpr16(0, val16); // AX
        Ok(())
    }

    /// MOV moffs64, AX
    /// Matching C++ data_xfer64.cc MOV_OqAX
    pub fn mov_oq_ax(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val16 = self.get_gpr16(0); // AX

        self.write_virtual_word_64(seg, instr.iq(), val16)?;
        Ok(())
    }

    /// MOV EAX, moffs64
    /// Matching C++ data_xfer64.cc MOV_EAXOq
    pub fn mov_eax_oq(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val32 = self.read_virtual_dword_64(seg, instr.iq())?;

        self.set_gpr32(0, val32); // EAX
        Ok(())
    }

    /// MOV moffs64, EAX
    /// Matching C++ data_xfer64.cc MOV_OqEAX
    pub fn mov_oq_eax(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val32 = self.get_gpr32(0); // EAX

        self.write_virtual_dword_64(seg, instr.iq(), val32)?;
        Ok(())
    }

    /// MOV RAX, moffs64
    /// Matching C++ data_xfer64.cc MOV_RAXOq
    pub fn mov_rax_oq(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val64 = self.read_virtual_qword_64(seg, instr.iq())?;

        self.set_gpr64(0, val64); // RAX
        Ok(())
    }

    /// MOV moffs64, RAX
    /// Matching C++ data_xfer64.cc MOV_OqRAX
    pub fn mov_oq_rax(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let val64 = self.get_gpr64(0); // RAX

        self.write_virtual_qword_64(seg, instr.iq(), val64)?;
        Ok(())
    }

    /// MOV r/m64, imm32 (sign-extended to 64-bit) (memory form)
    /// Matching C++ data_xfer64.cc MOV_EqIdM
    pub fn mov_eq_id_m(&mut self, instr: &Instruction) -> Result<()> {
        let op_64 = instr.id() as i32 as u64; // sign extend imm32 to 64-bit
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());

        self.write_virtual_qword_64(seg, eaddr, op_64)?;
        Ok(())
    }

    /// MOV r64, imm32 (sign-extended to 64-bit) (register form)
    /// Matching C++ data_xfer64.cc MOV_EqIdR
    pub fn mov_eq_id_r(&mut self, instr: &Instruction) {
        let op_64 = instr.id() as i32 as u64; // sign extend imm32 to 64-bit
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op_64);
    }

    // =========================================================================
    // MOVZX - Zero extend
    // =========================================================================

    /// MOVZX r64, r/m8 (memory form)
    /// Matching C++ data_xfer64.cc MOVZX_GqEbM
    /// Zero extend byte op2 into qword op1
    pub fn movzx_gq_eb_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_8 = self.read_virtual_byte_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_8 as u64);
        Ok(())
    }

    /// MOVZX r64, r8 (register form)
    /// Matching C++ data_xfer64.cc MOVZX_GqEbR
    /// Zero extend byte op2 into qword op1
    pub fn movzx_gq_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_8 as u64);
    }

    /// MOVZX r64, r/m16 (memory form)
    /// Matching C++ data_xfer64.cc MOVZX_GqEwM
    /// Zero extend word op2 into qword op1
    pub fn movzx_gq_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_16 as u64);
        Ok(())
    }

    /// MOVZX r64, r16 (register form)
    /// Matching C++ data_xfer64.cc MOVZX_GqEwR
    /// Zero extend word op2 into qword op1
    pub fn movzx_gq_ew_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_16 = self.get_gpr16(src_reg);
        let dst_reg = instr.dst() as usize;

        self.set_gpr64(dst_reg, op2_16 as u64);
    }

    // =========================================================================
    // MOVSX - Sign extend
    // =========================================================================

    /// MOVSX r64, r/m8 (memory form)
    /// Matching C++ data_xfer64.cc MOVSX_GqEbM
    /// Sign extend byte op2 into qword op1
    pub fn movsx_gq_eb_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_8 = self.read_virtual_byte_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_8 as i8 as i64) as u64; // sign extend byte to qword

        self.set_gpr64(dst_reg, val64);
        Ok(())
    }

    /// MOVSX r64, r8 (register form)
    /// Matching C++ data_xfer64.cc MOVSX_GqEbR
    /// Sign extend byte op2 into qword op1
    pub fn movsx_gq_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_8 as i8 as i64) as u64; // sign extend byte to qword

        self.set_gpr64(dst_reg, val64);
    }

    /// MOVSX r64, r/m16 (memory form)
    /// Matching C++ data_xfer64.cc MOVSX_GqEwM
    /// Sign extend word op2 into qword op1
    pub fn movsx_gq_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_16 as i16 as i64) as u64; // sign extend word to qword

        self.set_gpr64(dst_reg, val64);
        Ok(())
    }

    /// MOVSX r64, r16 (register form)
    /// Matching C++ data_xfer64.cc MOVSX_GqEwR
    /// Sign extend word op2 into qword op1
    pub fn movsx_gq_ew_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_16 = self.get_gpr16(src_reg);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_16 as i16 as i64) as u64; // sign extend word to qword

        self.set_gpr64(dst_reg, val64);
    }

    /// MOVSX r64, r/m32 (memory form)
    /// Matching C++ data_xfer64.cc MOVSX_GqEdM
    /// Sign extend dword op2 into qword op1
    pub fn movsx_gq_ed_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.read_virtual_dword_64(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_32 as i32 as i64) as u64; // sign extend dword to qword

        self.set_gpr64(dst_reg, val64);
        Ok(())
    }

    /// MOVSX r64, r32 (register form)
    /// Matching C++ data_xfer64.cc MOVSX_GqEdR
    /// Sign extend dword op2 into qword op1
    pub fn movsx_gq_ed_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let dst_reg = instr.dst() as usize;
        let val64 = (op2_32 as i32 as i64) as u64; // sign extend dword to qword

        self.set_gpr64(dst_reg, val64);
    }

    // =========================================================================
    // XCHG - Exchange
    // =========================================================================

    /// XCHG r/m64, r64 (memory form)
    /// Matching C++ data_xfer64.cc XCHG_EqGqM
    /// Note: always locked (read_RMW_virtual_qword)
    /// XCHG 0x87 is NOT in decoder swap list, so [0]=nnn=register, [1]=rm
    pub fn xchg_eq_gq_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_64 = self.read_rmw_virtual_qword_64(seg, eaddr)?;
        let src_reg = instr.dst() as usize; // dst()=[0]=nnn=register operand
        let op2_64 = self.get_gpr64(src_reg);

        self.write_rmw_virtual_qword_back_64(op2_64);
        self.set_gpr64(src_reg, op1_64);
        Ok(())
    }

    /// XCHG r64, r64 (register form)
    /// Matching C++ data_xfer64.cc XCHG_EqGqR
    pub fn xchg_eq_gq_r(&mut self, instr: &Instruction) {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;
        let op1_64 = self.get_gpr64(dst_reg);
        let op2_64 = self.get_gpr64(src_reg);

        self.set_gpr64(src_reg, op1_64);
        self.set_gpr64(dst_reg, op2_64);
    }

    /// XCHG r64, RAX — opcode 0x90+rd with REX.W
    /// Bochs: XCHG_RRXRax
    pub fn xchg_rrx_rax(&mut self, instr: &Instruction) {
        let reg = instr.dst() as usize;
        let val_rax = self.rax();
        let val_reg = self.get_gpr64(reg);
        self.set_rax(val_reg);
        self.set_gpr64(reg, val_rax);
    }

    // =========================================================================
    // CMOV - Conditional Move (64-bit)
    // =========================================================================
    // Note: CMOV accesses a memory source operand (read), regardless
    //       of whether condition is true or not.  Thus, exceptions may
    //       occur even if the MOV does not take place.
    // Matching C++ data_xfer64.cc

    /// Conditional move if overflow (OF=1)
    /// Matching C++ data_xfer64.cc CMOVO_GqEqR
    pub fn cmovo_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not overflow (OF=0)
    /// Matching C++ data_xfer64.cc CMOVNO_GqEqR
    pub fn cmovno_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if below/carry (CF=1)
    /// Matching C++ data_xfer64.cc CMOVB_GqEqR
    pub fn cmovb_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_cf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not below/no carry (CF=0)
    /// Matching C++ data_xfer64.cc CMOVNB_GqEqR
    pub fn cmovnb_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_cf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if zero/equal (ZF=1)
    /// Matching C++ data_xfer64.cc CMOVZ_GqEqR
    pub fn cmovz_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not zero/not equal (ZF=0)
    /// Matching C++ data_xfer64.cc CMOVNZ_GqEqR
    pub fn cmovnz_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if below or equal (CF=1 or ZF=1)
    /// Matching C++ data_xfer64.cc CMOVBE_GqEqR
    pub fn cmovbe_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_cf() || self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not below or equal/above (CF=0 and ZF=0)
    /// Matching C++ data_xfer64.cc CMOVNBE_GqEqR
    pub fn cmovnbe_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_cf() && !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if sign (SF=1)
    /// Matching C++ data_xfer64.cc CMOVS_GqEqR
    pub fn cmovs_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_sf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not sign (SF=0)
    /// Matching C++ data_xfer64.cc CMOVNS_GqEqR
    pub fn cmovns_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_sf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if parity/parity even (PF=1)
    /// Matching C++ data_xfer64.cc CMOVP_GqEqR
    pub fn cmovp_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_pf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if no parity/parity odd (PF=0)
    /// Matching C++ data_xfer64.cc CMOVNP_GqEqR
    pub fn cmovnp_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_pf() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if less (SF != OF)
    /// Matching C++ data_xfer64.cc CMOVL_GqEqR
    pub fn cmovl_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_sf() != self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not less/greater or equal (SF == OF)
    /// Matching C++ data_xfer64.cc CMOVNL_GqEqR
    pub fn cmovnl_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_sf() == self.get_of() {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if less or equal (ZF=1 or SF!=OF)
    /// Matching C++ data_xfer64.cc CMOVLE_GqEqR
    pub fn cmovle_gq_eq_r(&mut self, instr: &Instruction) {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    /// Conditional move if not less or equal/greater (ZF=0 and SF==OF)
    /// Matching C++ data_xfer64.cc CMOVNLE_GqEqR
    pub fn cmovnle_gq_eq_r(&mut self, instr: &Instruction) {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let src_reg = instr.src() as usize;
            let val64 = self.get_gpr64(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr64(dst_reg, val64);
        }
    }

    // =========================================================================
    // Unified dispatchers (mod_c0 routing for register vs memory)
    // =========================================================================

    pub fn mov_eq_gq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            let src = instr.src() as usize;
            let dst = instr.dst() as usize;
            self.set_gpr64(dst, self.get_gpr64(src));
            Ok(())
        } else {
            self.mov_eq_gq_m(instr)
        }
    }

    pub fn mov_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.mov_gq_eq_r(instr);
            Ok(())
        } else {
            self.mov_gq_eq_m(instr)
        }
    }

    pub fn mov_eq_id(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.mov_eq_id_r(instr);
            Ok(())
        } else {
            self.mov_eq_id_m(instr)
        }
    }

    pub fn xchg_eq_gq(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.xchg_eq_gq_r(instr);
            Ok(())
        } else {
            self.xchg_eq_gq_m(instr)
        }
    }

    pub fn movsx_gq_eb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.movsx_gq_eb_r(instr);
            Ok(())
        } else {
            self.movsx_gq_eb_m(instr)
        }
    }

    pub fn movsx_gq_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.movsx_gq_ew_r(instr);
            Ok(())
        } else {
            self.movsx_gq_ew_m(instr)
        }
    }

    pub fn movsxd_gq_ed(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.movsx_gq_ed_r(instr);
            Ok(())
        } else {
            self.movsx_gq_ed_m(instr)
        }
    }

    pub fn movzx_gq_eb(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.movzx_gq_eb_r(instr);
            Ok(())
        } else {
            self.movzx_gq_eb_m(instr)
        }
    }

    pub fn movzx_gq_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.movzx_gq_ew_r(instr);
            Ok(())
        } else {
            self.movzx_gq_ew_m(instr)
        }
    }

    // =========================================================================
    // CMOVcc unified dispatchers (64-bit) - handle both register and memory forms
    // For memory form: always read the value, then conditionally write
    // =========================================================================

    fn cmov_read_src64(&mut self, instr: &Instruction) -> Result<u64> {
        if instr.mod_c0() {
            Ok(self.get_gpr64(instr.src() as usize))
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)
        }
    }

    pub fn cmovo_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_of() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovno_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_of() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovb_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_cf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovnb_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_cf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovz_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_zf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovnz_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_zf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovbe_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_cf() || self.get_zf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovnbe_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_cf() && !self.get_zf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovs_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_sf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovns_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_sf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovp_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_pf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovnp_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_pf() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovl_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_sf() != self.get_of() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovnl_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_sf() == self.get_of() { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovle_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if self.get_zf() || (self.get_sf() != self.get_of()) { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }
    pub fn cmovnle_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let val = self.cmov_read_src64(instr)?;
        if !self.get_zf() && (self.get_sf() == self.get_of()) { self.set_gpr64(instr.dst() as usize, val); }
        Ok(())
    }

    // =========================================================================
    // Helper functions for 64-bit memory operations
    // =========================================================================

    /// Resolve effective address (64-bit addressing mode)
    /// Matching BX_CPU_RESOLVE_ADDR_64
    /// Made pub(crate) so it can be accessed from ctrl_xfer64.rs
    pub(crate) fn resolve_addr64(&self, instr: &Instruction) -> u64 {
        // Calculate: base + (index << scale) + displacement
        // base_reg: 0-15 = GPR, 16 = RIP (for RIP-relative), 19 = NIL (no base)
        // gen_reg[16] holds RIP (already advanced by ilen before execution),
        // gen_reg[19] = NIL register (always 0).
        // Matching Bochs: ResolveModrm reads gen_reg[base] directly.
        let base_reg = instr.sib_base() as usize;
        let mut eaddr = if base_reg < self.gen_reg.len() {
            self.get_gpr64(base_reg)
        } else {
            0
        };

        eaddr = eaddr.wrapping_add(instr.displ32s() as u64);

        let index_reg = instr.sib_index();
        if index_reg != 4 {
            // 4 means no index
            let index_val = if index_reg < 16 {
                self.get_gpr64(index_reg as usize)
            } else {
                0
            };
            let scale = instr.sib_scale();
            eaddr = eaddr.wrapping_add(index_val << scale);
        }

        eaddr
    }

    // read_8bit_regx is defined in logical8.rs to avoid duplicate definitions
}
