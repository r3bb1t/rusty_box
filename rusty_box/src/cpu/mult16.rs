//! 16-bit multiplication and division instructions for x86 CPU emulation
//!
//! Based on Bochs mult16.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
    error::{CpuError, Result},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 16-bit Multiplication and Division
    // =========================================================================

    /// MUL r/m16 - Unsigned multiply AX by r/m16, result in DX:AX
    /// Matching C++ mult16.cc:MUL_AXEwR
    pub fn mul_ax_ew_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("MUL16: AX ({:#06x}) * reg{} ({:#06x}) = DX:AX ({:#06x}:{:#06x})", op1, src_reg, op2, product_16h, product_16l);
        Ok(())
    }

    /// MUL r/m16 (memory form)
    /// Matching C++ mult16.cc:MUL_AXEwM
    pub fn mul_ax_ew_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr16(0); // AX
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_word(seg, eaddr)?;

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
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("MUL16 mem: AX ({:#06x}) * [{:?}:{:#x}] ({:#06x}) = DX:AX ({:#06x}:{:#06x})", op1, seg, eaddr, op2, product_16h, product_16l);
        Ok(())
    }

    /// IMUL r/m16 - Signed multiply AX by r/m16, result in DX:AX
    /// Matching C++ mult16.cc:IMUL_AXEwR
    pub fn imul_ax_ew_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL16: AX ({:#06x}) * reg{} ({:#06x}) = DX:AX ({:#06x}:{:#06x})", op1 as u16, src_reg, op2 as u16, product_16h, product_16l);
        Ok(())
    }

    /// IMUL r/m16 (memory form)
    /// Matching C++ mult16.cc:IMUL_AXEwM
    pub fn imul_ax_ew_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr16(0) as i16; // AX
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_word(seg, eaddr)? as i16;

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
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL16 mem: AX ({:#06x}) * [{:?}:{:#x}] ({:#06x}) = DX:AX ({:#06x}:{:#06x})", op1 as u16, seg, eaddr, op2 as u16, product_16h, product_16l);
        Ok(())
    }

    /// DIV r/m16 - Unsigned divide DX:AX by r/m16, quotient in AX, remainder in DX
    /// Matching C++ mult16.cc:DIV_AXEwR
    pub fn div_ax_ew_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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

        tracing::trace!("DIV16: DX:AX ({:#06x}:{:#06x}) / reg{} ({:#06x}) = AX ({:#06x}), DX ({:#06x})", dx, ax, src_reg, op2, quotient_16l, remainder_16);
        Ok(())
    }

    /// DIV r/m16 (memory form)
    /// Matching C++ mult16.cc:DIV_AXEwM
    pub fn div_ax_ew_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_word(seg, eaddr)?;

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

        tracing::trace!("DIV16 mem: DX:AX ({:#06x}:{:#06x}) / [{:?}:{:#x}] ({:#06x}) = AX ({:#06x}), DX ({:#06x})", dx, ax, seg, eaddr, op2, quotient_16l, remainder_16);
        Ok(())
    }

    /// IDIV r/m16 - Signed divide DX:AX by r/m16, quotient in AX, remainder in DX
    /// Matching C++ mult16.cc:IDIV_AXEwR
    pub fn idiv_ax_ew_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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

        tracing::trace!("IDIV16: DX:AX ({:#06x}:{:#06x}) / reg{} ({:#06x}) = AX ({:#06x}), DX ({:#06x})", dx as u16, ax as u16, src_reg, op2 as u16, quotient_16l as u16, remainder_16 as u16);
        Ok(())
    }

    /// IDIV r/m16 (memory form)
    /// Matching C++ mult16.cc:IDIV_AXEwM
    pub fn idiv_ax_ew_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let dx = self.get_gpr16(2); // DX
        let ax = self.get_gpr16(0); // AX
        // Matching C++: Bit32s op1_32 = ((((Bit32u) DX) << 16) | ((Bit32u) AX));
        // Construct as unsigned first, then cast to signed
        let op1 = (((dx as u32) << 16) | (ax as u32)) as i32;

        // Check MIN_INT case
        if op1 == 0x80000000u32 as i32 {
            return self.exception(Exception::De, 0);
        }

        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_word(seg, eaddr)? as i16;

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

        tracing::trace!("IDIV16 mem: DX:AX ({:#06x}:{:#06x}) / [{:?}:{:#x}] ({:#06x}) = AX ({:#06x}), DX ({:#06x})", dx as u16, ax as u16, seg, eaddr, op2 as u16, quotient_16l as u16, remainder_16 as u16);
        Ok(())
    }

    // =========================================================================
    // Unified wrappers (dispatch register vs memory form based on mod_c0)
    // =========================================================================

    /// MUL AX, r/m16 - Unified wrapper
    pub fn mul_ax_ew(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.mul_ax_ew_r(instr) } else { self.mul_ax_ew_m(instr) }
    }

    /// IMUL AX, r/m16 - Unified wrapper
    pub fn imul_ax_ew(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.imul_ax_ew_r(instr) } else { self.imul_ax_ew_m(instr) }
    }

    /// DIV AX, r/m16 - Unified wrapper
    pub fn div_ax_ew(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.div_ax_ew_r(instr) } else { self.div_ax_ew_m(instr) }
    }

    /// IDIV AX, r/m16 - Unified wrapper
    pub fn idiv_ax_ew(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.idiv_ax_ew_r(instr) } else { self.idiv_ax_ew_m(instr) }
    }

    // =========================================================================
    // Helper functions
    // =========================================================================

    // Helper methods (resolve_addr32, read_virtual_word) are defined in logical16.rs to avoid duplicate definitions
}
