//! 8-bit multiplication and division instructions for x86 CPU emulation
//!
//! Based on Bochs mult8.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
    error::{CpuError, Result},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 8-bit Multiplication and Division
    // =========================================================================

    /// MUL r/m8 - Unsigned multiply AL by r/m8, result in AX
    /// Matching C++ mult8.cc:27-48 MUL_ALEbR
    pub fn mul_al_eb_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr8(0); // AL
        let src_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);

        let product_16 = (op1 as u16) * (op2 as u16);
        let product_8l = (product_16 & 0xFF) as u8;
        let product_8h = (product_16 >> 8) as u8;

        // Write product to AX
        self.set_gpr16(0, product_16);

        // Set flags
        self.update_flags_logic8(product_8l);
        if product_8h != 0 {
            // Set CF and OF if high byte is non-zero
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("MUL8: AL ({:#04x}) * reg{} ({:#04x}) = AX ({:#06x})", op1, src_reg, op2, product_16);
        Ok(())
    }

    /// MUL r/m8 (memory form)
    /// Matching C++ mult8.cc:27-48 MUL_ALEbM
    pub fn mul_al_eb_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr8(0); // AL
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr)?;

        let product_16 = (op1 as u16) * (op2 as u16);
        let product_8l = (product_16 & 0xFF) as u8;
        let product_8h = (product_16 >> 8) as u8;

        // Write product to AX
        self.set_gpr16(0, product_16);

        // Set flags
        self.update_flags_logic8(product_8l);
        if product_8h != 0 {
            // Set CF and OF if high byte is non-zero
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("MUL8 mem: AL ({:#04x}) * [{:?}:{:#x}] ({:#04x}) = AX ({:#06x})", op1, seg, eaddr, op2, product_16);
        Ok(())
    }

    /// IMUL r/m8 - Signed multiply AL by r/m8, result in AX
    /// Matching C++ mult8.cc:50-73 IMUL_ALEbR
    pub fn imul_al_eb_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr8(0) as i8; // AL
        let src_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l) as i8;

        let product_16 = (op1 as i16) * (op2 as i16);
        let product_8 = (product_16 & 0xFF) as u8;

        // Write product to AX
        self.set_gpr16(0, product_16 as u16);

        // Set flags
        self.update_flags_logic8(product_8);
        // CF and OF are set if product_16 doesn't fit in signed 8-bit
        // Matching C++: if(product_16 != (Bit8s) product_16)
        // This checks if the 16-bit value equals its sign-extended 8-bit version
        if product_16 != (product_16 as i8 as i16) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL8: AL ({:#04x}) * reg{} ({:#04x}) = AX ({:#06x})", op1 as u8, src_reg, op2 as u8, product_16 as u16);
        Ok(())
    }

    /// IMUL r/m8 (memory form)
    /// Matching C++ mult8.cc:50-73 IMUL_ALEbM
    pub fn imul_al_eb_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr8(0) as i8; // AL
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr)? as i8;

        let product_16 = (op1 as i16) * (op2 as i16);
        let product_8 = (product_16 & 0xFF) as u8;

        // Write product to AX
        self.set_gpr16(0, product_16 as u16);

        // Set flags
        self.update_flags_logic8(product_8);
        // CF and OF are set if product_16 doesn't fit in signed 8-bit
        // Matching C++: if(product_16 != (Bit8s) product_16)
        // This checks if the 16-bit value equals its sign-extended 8-bit version
        if product_16 != (product_16 as i8 as i16) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL8 mem: AL ({:#04x}) * [{:?}:{:#x}] ({:#04x}) = AX ({:#06x})", op1 as u8, seg, eaddr, op2 as u8, product_16 as u16);
        Ok(())
    }

    /// DIV r/m8 - Unsigned divide AX by r/m8, quotient in AL, remainder in AH
    /// Matching C++ mult8.cc:75-98 DIV_ALEbR
    pub fn div_al_eb_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let src_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let op1 = self.get_gpr16(0); // AX
        let quotient_16 = op1 / (op2 as u16);
        let remainder_8 = (op1 % (op2 as u16)) as u8;
        let quotient_8l = (quotient_16 & 0xFF) as u8;

        if quotient_16 != (quotient_8l as u16) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AL, remainder to AH
        self.set_gpr8(0, quotient_8l); // AL
        self.set_gpr8(4, remainder_8); // AH (reg 4 = AH)

        tracing::trace!("DIV8: AX ({:#06x}) / reg{} ({:#04x}) = AL ({:#04x}), AH ({:#04x})", op1, src_reg, op2, quotient_8l, remainder_8);
        Ok(())
    }

    /// DIV r/m8 (memory form)
    /// Matching C++ mult8.cc:75-98 DIV_ALEbM
    pub fn div_al_eb_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr)?;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let op1 = self.get_gpr16(0); // AX
        let quotient_16 = op1 / (op2 as u16);
        let remainder_8 = (op1 % (op2 as u16)) as u8;
        let quotient_8l = (quotient_16 & 0xFF) as u8;

        if quotient_16 != (quotient_8l as u16) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AL, remainder to AH
        self.set_gpr8(0, quotient_8l); // AL
        self.set_gpr8(4, remainder_8); // AH (reg 4 = AH)

        tracing::trace!("DIV8 mem: AX ({:#06x}) / [{:?}:{:#x}] ({:#04x}) = AL ({:#04x}), AH ({:#04x})", op1, seg, eaddr, op2, quotient_8l, remainder_8);
        Ok(())
    }

    /// IDIV r/m8 - Signed divide AX by r/m8, quotient in AL, remainder in AH
    /// Matching C++ mult8.cc:100-125 IDIV_ALEbR
    pub fn idiv_al_eb_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr16(0) as i16; // AX

        // Check MIN_INT case
        if op1 == 0x8000u16 as i16 {
            return self.exception(Exception::De, 0);
        }

        let src_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l) as i8;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let quotient_16 = op1 / (op2 as i16);
        let remainder_8 = (op1 % (op2 as i16)) as i8 as u8;
        let quotient_8l = (quotient_16 & 0xFF) as i8;

        if quotient_16 != (quotient_8l as i16) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AL, remainder to AH
        self.set_gpr8(0, quotient_8l as u8); // AL
        self.set_gpr8(4, remainder_8); // AH (reg 4 = AH)

        tracing::trace!("IDIV8: AX ({:#06x}) / reg{} ({:#04x}) = AL ({:#04x}), AH ({:#04x})", op1 as u16, src_reg, op2 as u8, quotient_8l as u8, remainder_8);
        Ok(())
    }

    /// IDIV r/m8 (memory form)
    /// Matching C++ mult8.cc:100-125 IDIV_ALEbM
    pub fn idiv_al_eb_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr16(0) as i16; // AX

        // Check MIN_INT case
        if op1 == 0x8000u16 as i16 {
            return self.exception(Exception::De, 0);
        }

        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr)? as i8;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let quotient_16 = op1 / (op2 as i16);
        let remainder_8 = (op1 % (op2 as i16)) as i8 as u8;
        let quotient_8l = (quotient_16 & 0xFF) as i8;

        if quotient_16 != (quotient_8l as i16) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to AL, remainder to AH
        self.set_gpr8(0, quotient_8l as u8); // AL
        self.set_gpr8(4, remainder_8); // AH (reg 4 = AH)

        tracing::trace!("IDIV8 mem: AX ({:#06x}) / [{:?}:{:#x}] ({:#04x}) = AL ({:#04x}), AH ({:#04x})", op1 as u16, seg, eaddr, op2 as u8, quotient_8l as u8, remainder_8);
        Ok(())
    }

    // =========================================================================
    // Unified wrappers (dispatch register vs memory form based on mod_c0)
    // =========================================================================

    /// MUL AL, r/m8 - Unified wrapper
    pub fn mul_al_eb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.mul_al_eb_r(instr) } else { self.mul_al_eb_m(instr) }
    }

    /// IMUL AL, r/m8 - Unified wrapper
    pub fn imul_al_eb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.imul_al_eb_r(instr) } else { self.imul_al_eb_m(instr) }
    }

    /// DIV AL, r/m8 - Unified wrapper
    pub fn div_al_eb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.div_al_eb_r(instr) } else { self.div_al_eb_m(instr) }
    }

    /// IDIV AL, r/m8 - Unified wrapper
    pub fn idiv_al_eb(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.idiv_al_eb_r(instr) } else { self.idiv_al_eb_m(instr) }
    }

    // =========================================================================
    // Helper functions
    // =========================================================================

    // Helper methods (resolve_addr32, read_8bit_regx, read_virtual_byte) are defined in logical8.rs
}
