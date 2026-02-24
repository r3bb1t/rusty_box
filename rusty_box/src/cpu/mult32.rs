//! 32-bit multiplication and division instructions for x86 CPU emulation
//!
//! Based on Bochs mult32.cc
//! Copyright (C) 2001-2018 The Bochs Project

use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
    error::{CpuError, Result},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // 32-bit Multiplication and Division
    // =========================================================================

    /// MUL r/m32 - Unsigned multiply EAX by r/m32, result in EDX:EAX
    /// Matching C++ mult32.cc:MUL_EAXEdR
    pub fn mul_eax_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr32(0); // EAX
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr32(src_reg);

        let product_64 = (op1 as u64) * (op2 as u64);
        let product_32l = (product_64 & 0xFFFFFFFF) as u32;
        let product_32h = (product_64 >> 32) as u32;

        // Write product to EDX:EAX
        self.set_gpr32(0, product_32l); // EAX
        self.set_gpr32(2, product_32h); // EDX (reg 2 = EDX)

        // Set flags
        self.update_flags_logic32(product_32l);
        if product_32h != 0 {
            // Set CF and OF if high dword is non-zero
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("MUL32: EAX ({:#010x}) * reg{} ({:#010x}) = EDX:EAX ({:#010x}:{:#010x})", op1, src_reg, op2, product_32h, product_32l);
        Ok(())
    }

    /// MUL r/m32 (memory form)
    /// Matching C++ mult32.cc:MUL_EAXEdM
    pub fn mul_eax_ed_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr32(0); // EAX
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_dword(seg, eaddr);

        let product_64 = (op1 as u64) * (op2 as u64);
        let product_32l = (product_64 & 0xFFFFFFFF) as u32;
        let product_32h = (product_64 >> 32) as u32;

        // Write product to EDX:EAX
        self.set_gpr32(0, product_32l); // EAX
        self.set_gpr32(2, product_32h); // EDX (reg 2 = EDX)

        // Set flags
        self.update_flags_logic32(product_32l);
        if product_32h != 0 {
            // Set CF and OF if high dword is non-zero
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("MUL32 mem: EAX ({:#010x}) * [{:?}:{:#x}] ({:#010x}) = EDX:EAX ({:#010x}:{:#010x})", op1, seg, eaddr, op2, product_32h, product_32l);
        Ok(())
    }

    /// IMUL r/m32 - Signed multiply EAX by r/m32, result in EDX:EAX
    /// Matching C++ mult32.cc:IMUL_EAXEdR
    pub fn imul_eax_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr32(0) as i32; // EAX
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr32(src_reg) as i32;

        let product_64 = (op1 as i64) * (op2 as i64);
        let product_32l = (product_64 & 0xFFFFFFFF) as u32;
        let product_32h = ((product_64 >> 32) & 0xFFFFFFFF) as u32;

        // Write product to EDX:EAX
        self.set_gpr32(0, product_32l); // EAX
        self.set_gpr32(2, product_32h); // EDX (reg 2 = EDX)

        // Set flags
        self.update_flags_logic32(product_32l);
        // CF and OF are set if product_64 doesn't fit in signed 32-bit
        // Matching C++: if(product_64 != (Bit32s)product_64)
        // This checks if the 64-bit value equals its sign-extended 32-bit version
        if product_64 != (product_64 as i32 as i64) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL32: EAX ({:#010x}) * reg{} ({:#010x}) = EDX:EAX ({:#010x}:{:#010x})", op1 as u32, src_reg, op2 as u32, product_32h, product_32l);
        Ok(())
    }

    /// IMUL r/m32 (memory form)
    /// Matching C++ mult32.cc:IMUL_EAXEdM
    pub fn imul_eax_ed_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let op1 = self.get_gpr32(0) as i32; // EAX
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_dword(seg, eaddr) as i32;

        let product_64 = (op1 as i64) * (op2 as i64);
        let product_32l = (product_64 & 0xFFFFFFFF) as u32;
        let product_32h = ((product_64 >> 32) & 0xFFFFFFFF) as u32;

        // Write product to EDX:EAX
        self.set_gpr32(0, product_32l); // EAX
        self.set_gpr32(2, product_32h); // EDX (reg 2 = EDX)

        // Set flags
        self.update_flags_logic32(product_32l);
        // CF and OF are set if product_64 doesn't fit in signed 32-bit
        // Matching C++: if(product_64 != (Bit32s)product_64)
        // This checks if the 64-bit value equals its sign-extended 32-bit version
        if product_64 != (product_64 as i32 as i64) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL32 mem: EAX ({:#010x}) * [{:?}:{:#x}] ({:#010x}) = EDX:EAX ({:#010x}:{:#010x})", op1 as u32, seg, eaddr, op2 as u32, product_32h, product_32l);
        Ok(())
    }

    /// DIV r/m32 - Unsigned divide EDX:EAX by r/m32, quotient in EAX, remainder in EDX
    /// Matching C++ mult32.cc:DIV_EAXEdR
    pub fn div_eax_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr32(src_reg);

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let edx = self.get_gpr32(2); // EDX
        let eax = self.get_gpr32(0); // EAX
        let op1 = ((edx as u64) << 32) | (eax as u64);

        let quotient_64 = op1 / (op2 as u64);
        let remainder_32 = (op1 % (op2 as u64)) as u32;
        let quotient_32l = (quotient_64 & 0xFFFFFFFF) as u32;

        if quotient_64 != (quotient_32l as u64) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to EAX, remainder to EDX
        self.set_gpr32(0, quotient_32l); // EAX
        self.set_gpr32(2, remainder_32); // EDX

        tracing::trace!("DIV32: EDX:EAX ({:#010x}:{:#010x}) / reg{} ({:#010x}) = EAX ({:#010x}), EDX ({:#010x})", edx, eax, src_reg, op2, quotient_32l, remainder_32);
        Ok(())
    }

    /// DIV r/m32 (memory form)
    /// Matching C++ mult32.cc:DIV_EAXEdM
    pub fn div_eax_ed_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_dword(seg, eaddr);

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let edx = self.get_gpr32(2); // EDX
        let eax = self.get_gpr32(0); // EAX
        let op1 = ((edx as u64) << 32) | (eax as u64);

        let quotient_64 = op1 / (op2 as u64);
        let remainder_32 = (op1 % (op2 as u64)) as u32;
        let quotient_32l = (quotient_64 & 0xFFFFFFFF) as u32;

        if quotient_64 != (quotient_32l as u64) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to EAX, remainder to EDX
        self.set_gpr32(0, quotient_32l); // EAX
        self.set_gpr32(2, remainder_32); // EDX

        tracing::trace!("DIV32 mem: EDX:EAX ({:#010x}:{:#010x}) / [{:?}:{:#x}] ({:#010x}) = EAX ({:#010x}), EDX ({:#010x})", edx, eax, seg, eaddr, op2, quotient_32l, remainder_32);
        Ok(())
    }

    /// IDIV r/m32 - Signed divide EDX:EAX by r/m32, quotient in EAX, remainder in EDX
    /// Matching C++ mult32.cc:IDIV_EAXEdR
    pub fn idiv_eax_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let edx = self.get_gpr32(2); // EDX
        let eax = self.get_gpr32(0); // EAX
        // Matching C++: Bit64s op1_64 = GET64_FROM_HI32_LO32(EDX, EAX);
        // GET64_FROM_HI32_LO32 is: (Bit64u(lo) | (Bit64u(hi) << 32))
        // Construct as unsigned first, then cast to signed
        let op1 = ((eax as u64) | ((edx as u64) << 32)) as i64;

        // Check MIN_INT case
        if op1 == 0x8000000000000000u64 as i64 {
            return self.exception(Exception::De, 0);
        }

        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr32(src_reg) as i32;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let quotient_64 = op1 / (op2 as i64);
        let remainder_32 = (op1 % (op2 as i64)) as i32;
        let quotient_32l = (quotient_64 & 0xFFFFFFFF) as i32;

        if quotient_64 != (quotient_32l as i64) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to EAX, remainder to EDX
        self.set_gpr32(0, quotient_32l as u32); // EAX
        self.set_gpr32(2, remainder_32 as u32); // EDX

        tracing::trace!("IDIV32: EDX:EAX ({:#010x}:{:#010x}) / reg{} ({:#010x}) = EAX ({:#010x}), EDX ({:#010x})", edx as u32, eax as u32, src_reg, op2 as u32, quotient_32l as u32, remainder_32 as u32);
        Ok(())
    }

    /// IDIV r/m32 (memory form)
    /// Matching C++ mult32.cc:IDIV_EAXEdM
    pub fn idiv_eax_ed_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let edx = self.get_gpr32(2); // EDX
        let eax = self.get_gpr32(0); // EAX
        // Matching C++: Bit64s op1_64 = GET64_FROM_HI32_LO32(EDX, EAX);
        // GET64_FROM_HI32_LO32 is: (Bit64u(lo) | (Bit64u(hi) << 32))
        // Construct as unsigned first, then cast to signed
        let op1 = ((eax as u64) | ((edx as u64) << 32)) as i64;

        // Check MIN_INT case
        if op1 == 0x8000000000000000u64 as i64 {
            return self.exception(Exception::De, 0);
        }

        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_dword(seg, eaddr) as i32;

        if op2 == 0 {
            return self.exception(Exception::De, 0);
        }

        let quotient_64 = op1 / (op2 as i64);
        let remainder_32 = (op1 % (op2 as i64)) as i32;
        let quotient_32l = (quotient_64 & 0xFFFFFFFF) as i32;

        if quotient_64 != (quotient_32l as i64) {
            return self.exception(Exception::De, 0);
        }

        // Write quotient to EAX, remainder to EDX
        self.set_gpr32(0, quotient_32l as u32); // EAX
        self.set_gpr32(2, remainder_32 as u32); // EDX

        tracing::trace!("IDIV32 mem: EDX:EAX ({:#010x}:{:#010x}) / [{:?}:{:#x}] ({:#010x}) = EAX ({:#010x}), EDX ({:#010x})", edx as u32, eax as u32, seg, eaddr, op2 as u32, quotient_32l as u32, remainder_32 as u32);
        Ok(())
    }

    /// IMUL Gd, Ed - Two-operand signed multiply (register form)
    /// dst = dst * src, only lower 32 bits stored
    /// Matching C++ mult32.cc:IMUL_GdEdR
    pub fn imul_gd_ed_r(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        let op1 = self.get_gpr32(dst_reg) as i32;
        let op2 = self.get_gpr32(src_reg) as i32;

        let product_64 = (op1 as i64) * (op2 as i64);
        let product_32 = (product_64 & 0xFFFFFFFF) as u32;

        self.set_gpr32(dst_reg, product_32);

        self.update_flags_logic32(product_32);
        if product_64 != (product_64 as i32 as i64) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL Gd,Ed: reg{} ({:#010x}) * reg{} ({:#010x}) = {:#010x}",
            dst_reg, op1 as u32, src_reg, op2 as u32, product_32);
        Ok(())
    }

    /// IMUL Gd, Ed - Two-operand signed multiply (memory form)
    /// Matching C++ mult32.cc:IMUL_GdEdM
    pub fn imul_gd_ed_m(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let dst_reg = instr.dst() as usize;

        let op1 = self.get_gpr32(dst_reg) as i32;
        let eaddr = self.resolve_addr32(instr);
        let seg = super::decoder::BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_dword(seg, eaddr) as i32;

        let product_64 = (op1 as i64) * (op2 as i64);
        let product_32 = (product_64 & 0xFFFFFFFF) as u32;

        self.set_gpr32(dst_reg, product_32);

        self.update_flags_logic32(product_32);
        if product_64 != (product_64 as i32 as i64) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        }

        tracing::trace!("IMUL Gd,Ed mem: reg{} ({:#010x}) * [{:?}:{:#x}] ({:#010x}) = {:#010x}",
            dst_reg, op1 as u32, seg, eaddr, op2 as u32, product_32);
        Ok(())
    }

    /// IMUL Gd, Ed, Ib - Three-operand signed multiply with 8-bit immediate
    /// dst = src * sign_extend(imm8)
    /// Opcode: 6B /r ib
    pub fn imul_gd_ed_ib(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        let dst_reg = instr.dst() as usize;
        let src_reg = instr.src() as usize;

        // Get source operand (32-bit)
        let op1 = self.get_gpr32(src_reg) as i32;

        // Get immediate operand (8-bit, sign-extended to 32-bit)
        let imm8 = instr.ib() as i8;
        let op2 = imm8 as i32;

        // Perform signed multiplication
        let product_64 = (op1 as i64) * (op2 as i64);
        let result_32 = product_64 as i32;

        // Store result in destination register
        self.set_gpr32(dst_reg, result_32 as u32);

        // Set CF and OF if result doesn't fit in signed 32-bit
        // (i.e., if sign-extension of result doesn't equal the full 64-bit product)
        if product_64 != (result_32 as i64) {
            self.eflags |= (1 << 0) | (1 << 11); // CF=1, OF=1
        } else {
            self.eflags &= !((1 << 0) | (1 << 11)); // CF=0, OF=0
        }

        tracing::trace!("IMUL32: reg{} ({:#010x}) * imm8 ({:#04x}) = reg{} ({:#010x})",
            src_reg, op1 as u32, imm8 as u8, dst_reg, result_32 as u32);
        Ok(())
    }

    // =========================================================================
    // Unified wrappers (dispatch register vs memory form based on mod_c0)
    // =========================================================================

    /// MUL EAX, r/m32 - Unified wrapper
    pub fn mul_eax_ed(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.mul_eax_ed_r(instr) } else { self.mul_eax_ed_m(instr) }
    }

    /// IMUL EAX, r/m32 - Unified wrapper
    pub fn imul_eax_ed(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.imul_eax_ed_r(instr) } else { self.imul_eax_ed_m(instr) }
    }

    /// DIV EAX, r/m32 - Unified wrapper
    pub fn div_eax_ed(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.div_eax_ed_r(instr) } else { self.div_eax_ed_m(instr) }
    }

    /// IDIV EAX, r/m32 - Unified wrapper
    pub fn idiv_eax_ed(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
        if instr.mod_c0() { self.idiv_eax_ed_r(instr) } else { self.idiv_eax_ed_m(instr) }
    }

    // =========================================================================
    // Helper functions
    // =========================================================================

    // Helper method (resolve_addr32) is defined in logical32.rs to avoid duplicate definitions

    // read_virtual_dword is defined in logical32.rs to avoid duplicate definitions
}
