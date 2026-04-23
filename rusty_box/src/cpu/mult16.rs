//! 16-bit multiplication and division instructions for x86 CPU emulation
//!
//! Based on Bochs mult16.cc

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    error::Result,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // 16-bit Multiplication and Division
    // =========================================================================

    /// MUL r/m16 - Unsigned multiply AX by r/m16, result in DX:AX
    /// Matching C++ mult16.cc:MUL_AXEwR
    pub fn mul_ax_ew_r(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.get_gpr16(0); // AX
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr16(src_reg);

        let product_32 = (op1 as u32) * (op2 as u32);
        let product_16l = (product_32 & 0xFFFF) as u16;
        let product_16h = (product_32 >> 16) as u16;

        // Write product to DX:AX
        self.set_gpr16(0, product_16l); // AX
        self.set_gpr16(2, product_16h); // DX (reg 2 = DX)

        // Set flags
        self.update_flags_logic16(product_16l);
        if product_16h != 0 {
            // Set CF and OF if high word is non-zero
            self.oszapc.set_flags_oxxxxc(1, 1); // CF=1, OF=1
        }

        Ok(())
    }

    /// MUL r/m16 (memory form)
    /// Matching C++ mult16.cc:MUL_AXEwM
    pub fn mul_ax_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.get_gpr16(0); // AX
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.v_read_word(seg, eaddr)?;

        let product_32 = (op1 as u32) * (op2 as u32);
        let product_16l = (product_32 & 0xFFFF) as u16;
        let product_16h = (product_32 >> 16) as u16;

        // Write product to DX:AX
        self.set_gpr16(0, product_16l); // AX
        self.set_gpr16(2, product_16h); // DX (reg 2 = DX)

        // Set flags
        self.update_flags_logic16(product_16l);
        if product_16h != 0 {
            // Set CF and OF if high word is non-zero
            self.oszapc.set_flags_oxxxxc(1, 1); // CF=1, OF=1
        }

        Ok(())
    }

    /// IMUL r/m16 - Signed multiply AX by r/m16, result in DX:AX
    /// Matching C++ mult16.cc:IMUL_AXEwR
    pub fn imul_ax_ew_r(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.get_gpr16(0) as i16; // AX
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr16(src_reg) as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let product_16l = (product_32 & 0xFFFF) as u16;
        let product_16h = ((product_32 >> 16) & 0xFFFF) as u16;

        // Write product to DX:AX
        self.set_gpr16(0, product_16l); // AX
        self.set_gpr16(2, product_16h); // DX (reg 2 = DX)

        // Set flags
        self.update_flags_logic16(product_16l);
        // CF and OF are set if product_32 doesn't fit in signed 16-bit
        // Matching C++: if(product_32 != (Bit16s)product_32)
        // This checks if the 32-bit value equals its sign-extended 16-bit version
        if product_32 != (product_32 as i16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1); // CF=1, OF=1
        }

        Ok(())
    }

    /// IMUL r/m16 (memory form)
    /// Matching C++ mult16.cc:IMUL_AXEwM
    pub fn imul_ax_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let op1 = self.get_gpr16(0) as i16; // AX
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.v_read_word(seg, eaddr)? as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let product_16l = (product_32 & 0xFFFF) as u16;
        let product_16h = ((product_32 >> 16) & 0xFFFF) as u16;

        // Write product to DX:AX
        self.set_gpr16(0, product_16l); // AX
        self.set_gpr16(2, product_16h); // DX (reg 2 = DX)

        // Set flags
        self.update_flags_logic16(product_16l);
        // CF and OF are set if product_32 doesn't fit in signed 16-bit
        // Matching C++: if(product_32 != (Bit16s)product_32)
        // This checks if the 32-bit value equals its sign-extended 16-bit version
        if product_32 != (product_32 as i16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1); // CF=1, OF=1
        }

        Ok(())
    }

    /// DIV r/m16 - Unsigned divide DX:AX by r/m16, quotient in AX, remainder in DX
    /// Matching C++ mult16.cc:DIV_AXEwR
    pub fn div_ax_ew_r(&mut self, instr: &Instruction) -> Result<()> {
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr16(src_reg);

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let dx = self.get_gpr16(2); // DX
        let ax = self.get_gpr16(0); // AX
        let op1 = ((dx as u32) << 16) | (ax as u32);

        let quotient_32 = op1 / (op2 as u32);
        let remainder_16 = (op1 % (op2 as u32)) as u16;
        let quotient_16l = (quotient_32 & 0xFFFF) as u16;

        if quotient_32 != (quotient_16l as u32) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AX, remainder to DX
        self.set_gpr16(0, quotient_16l); // AX
        self.set_gpr16(2, remainder_16); // DX

        Ok(())
    }

    /// DIV r/m16 (memory form)
    /// Matching C++ mult16.cc:DIV_AXEwM
    pub fn div_ax_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.v_read_word(seg, eaddr)?;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let dx = self.get_gpr16(2); // DX
        let ax = self.get_gpr16(0); // AX
        let op1 = ((dx as u32) << 16) | (ax as u32);

        let quotient_32 = op1 / (op2 as u32);
        let remainder_16 = (op1 % (op2 as u32)) as u16;
        let quotient_16l = (quotient_32 & 0xFFFF) as u16;

        if quotient_32 != (quotient_16l as u32) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AX, remainder to DX
        self.set_gpr16(0, quotient_16l); // AX
        self.set_gpr16(2, remainder_16); // DX

        Ok(())
    }

    /// IDIV r/m16 - Signed divide DX:AX by r/m16, quotient in AX, remainder in DX
    /// Matching C++ mult16.cc:IDIV_AXEwR
    pub fn idiv_ax_ew_r(&mut self, instr: &Instruction) -> Result<()> {
        let dx = self.get_gpr16(2); // DX
        let ax = self.get_gpr16(0); // AX
                                    // Matching C++: Bit32s op1_32 = ((((Bit32u) DX) << 16) | ((Bit32u) AX));
                                    // Construct as unsigned first, then cast to signed
        let op1 = (((dx as u32) << 16) | (ax as u32)) as i32;

        // Check MIN_INT case
        if op1 == 0x80000000u32 as i32 {
            return self.exception(Exception::De, 0);
        }

        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr16(src_reg) as i16;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let quotient_32 = op1 / (op2 as i32);
        let remainder_16 = (op1 % (op2 as i32)) as i16;
        let quotient_16l = (quotient_32 & 0xFFFF) as i16;

        if quotient_32 != (quotient_16l as i32) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AX, remainder to DX
        self.set_gpr16(0, quotient_16l as u16); // AX
        self.set_gpr16(2, remainder_16 as u16); // DX

        Ok(())
    }

    /// IDIV r/m16 (memory form)
    /// Matching C++ mult16.cc:IDIV_AXEwM
    pub fn idiv_ax_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let dx = self.get_gpr16(2); // DX
        let ax = self.get_gpr16(0); // AX
                                    // Matching C++: Bit32s op1_32 = ((((Bit32u) DX) << 16) | ((Bit32u) AX));
                                    // Construct as unsigned first, then cast to signed
        let op1 = (((dx as u32) << 16) | (ax as u32)) as i32;

        // Check MIN_INT case
        if op1 == 0x80000000u32 as i32 {
            return self.exception(Exception::De, 0);
        }

        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.v_read_word(seg, eaddr)? as i16;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let quotient_32 = op1 / (op2 as i32);
        let remainder_16 = (op1 % (op2 as i32)) as i16;
        let quotient_16l = (quotient_32 & 0xFFFF) as i16;

        if quotient_32 != (quotient_16l as i32) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AX, remainder to DX
        self.set_gpr16(0, quotient_16l as u16); // AX
        self.set_gpr16(2, remainder_16 as u16); // DX

        Ok(())
    }

    /// IMUL Gw, Ew - Two-operand signed multiply 16-bit (register form)
    /// dst = dst * src, only lower 16 bits stored
    /// Opcode: 0F AF /r with OPSIZE prefix
    pub fn imul_gw_ew_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr16(dst_reg) as i16;
        let op2 = self.get_gpr16(src_reg) as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let result_16 = product_32 as i16;

        self.set_gpr16(dst_reg, result_16 as u16);

        if product_32 != (result_16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1); // CF=1, OF=1
        } else {
            self.oszapc.set_flags_oxxxxc(0, 0); // CF=0, OF=0
        }

        Ok(())
    }

    /// IMUL Gw, Ew - Two-operand signed multiply 16-bit (memory form)
    pub fn imul_gw_ew_m(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let op1 = self.get_gpr16(dst_reg) as i16;
        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.v_read_word(seg, eaddr)? as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let result_16 = product_32 as i16;

        self.set_gpr16(dst_reg, result_16 as u16);

        if product_32 != (result_16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1); // CF=1, OF=1
        } else {
            self.oszapc.set_flags_oxxxxc(0, 0); // CF=0, OF=0
        }

        Ok(())
    }

    /// IMUL Gw, Ew, Iw - Three-operand signed multiply with 16-bit immediate (register form)
    /// dst = src * imm16
    pub fn imul_gw_ew_iw_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr16(src_reg) as i16;
        let op2 = instr.iw() as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let result_16 = product_32 as i16;

        self.set_gpr16(dst_reg, result_16 as u16);

        if product_32 != (result_16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1);
        } else {
            self.oszapc.set_flags_oxxxxc(0, 0);
        }

        Ok(())
    }

    /// IMUL Gw, Ew, Iw - Three-operand signed multiply with 16-bit immediate (memory form)
    pub fn imul_gw_ew_iw_m(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op1 = self.v_read_word(seg, eaddr)? as i16;
        let op2 = instr.iw() as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let result_16 = product_32 as i16;

        self.set_gpr16(dst_reg, result_16 as u16);

        if product_32 != (result_16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1);
        } else {
            self.oszapc.set_flags_oxxxxc(0, 0);
        }

        Ok(())
    }

    /// IMUL Gw, Ew, sIb - Three-operand signed multiply with sign-extended 8-bit imm (register form)
    pub fn imul_gw_ew_sib_r(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr16(src_reg) as i16;
        let op2 = instr.ib() as i8 as i16; // sign-extend 8→16

        let product_32 = (op1 as i32) * (op2 as i32);
        let result_16 = product_32 as i16;

        self.set_gpr16(dst_reg, result_16 as u16);

        if product_32 != (result_16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1);
        } else {
            self.oszapc.set_flags_oxxxxc(0, 0);
        }

        Ok(())
    }

    /// IMUL Gw, Ew, sIb - Three-operand signed multiply with sign-extended 8-bit imm (memory form)
    pub fn imul_gw_ew_sib_m(&mut self, instr: &Instruction) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let eaddr = self.resolve_addr(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op1 = self.v_read_word(seg, eaddr)? as i16;
        let op2 = instr.ib() as i8 as i16;

        let product_32 = (op1 as i32) * (op2 as i32);
        let result_16 = product_32 as i16;

        self.set_gpr16(dst_reg, result_16 as u16);

        if product_32 != (result_16 as i32) {
            self.oszapc.set_flags_oxxxxc(1, 1);
        } else {
            self.oszapc.set_flags_oxxxxc(0, 0);
        }

        Ok(())
    }

    // =========================================================================
    // Unified wrappers (dispatch register vs memory form based on mod_c0)
    // =========================================================================

    /// MUL AX, r/m16 - Unified wrapper
    pub fn mul_ax_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.mul_ax_ew_r(instr)
        } else {
            self.mul_ax_ew_m(instr)
        }
    }

    /// IMUL AX, r/m16 - Unified wrapper
    pub fn imul_ax_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_ax_ew_r(instr)
        } else {
            self.imul_ax_ew_m(instr)
        }
    }

    /// DIV AX, r/m16 - Unified wrapper
    pub fn div_ax_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.div_ax_ew_r(instr)
        } else {
            self.div_ax_ew_m(instr)
        }
    }

    /// IDIV AX, r/m16 - Unified wrapper
    pub fn idiv_ax_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.idiv_ax_ew_r(instr)
        } else {
            self.idiv_ax_ew_m(instr)
        }
    }

    /// IMUL Gw, Ew - Two-operand signed multiply 16-bit - Unified wrapper
    pub fn imul_gw_ew(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_gw_ew_r(instr)
        } else {
            self.imul_gw_ew_m(instr)
        }
    }

    /// IMUL Gw, Ew, Iw - Three-operand with 16-bit immediate - Unified wrapper
    pub fn imul_gw_ew_iw(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_gw_ew_iw_r(instr)
        } else {
            self.imul_gw_ew_iw_m(instr)
        }
    }

    /// IMUL Gw, Ew, sIb - Three-operand with sign-extended 8-bit immediate - Unified wrapper
    pub fn imul_gw_ew_sib(&mut self, instr: &Instruction) -> Result<()> {
        if instr.mod_c0() {
            self.imul_gw_ew_sib_r(instr)
        } else {
            self.imul_gw_ew_sib_m(instr)
        }
    }

    // =========================================================================
    // Helper functions
    // =========================================================================

    // Helper methods (resolve_addr, v_read_word) are defined in logical16.rs to avoid duplicate definitions
}
