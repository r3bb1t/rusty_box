//! 32-bit shift and rotate instructions
//!
//! Based on Bochs shift32.cc
//!
//! Implements SHL, SHR, SAR, ROL, ROR, SHLD, SHRD for 32-bit operands

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Bochs BX_CLEAR_64BIT_HIGH — called on count==0 for register form.
    /// In 64-bit mode, 32-bit register writes zero-extend. Even a no-op shift
    /// with count=0 still "writes" the register, clearing upper 32 bits.
    #[inline]
    fn shift32_count0_clear(&mut self, instr: &Instruction) {
        if instr.mod_c0() {
            self.bx_clear_64bit_high(instr.dst() as usize);
        }
    }

    // ---- 32-bit read/write helpers for shift instructions ----
    fn shift_read32(&mut self, instr: &Instruction) -> super::Result<(u32, Option<()>)> {
        if instr.mod_c0() {
            Ok((self.get_gpr32(instr.dst() as usize), None))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_rmw_dword(seg, eaddr)?;
            Ok((val, Some(())))
        }
    }
    fn shift_write32(&mut self, instr: &Instruction, paddr: Option<()>, result: u32) {
        if paddr.is_some() {
            self.write_rmw_linear_dword(result);
        } else {
            self.set_gpr32(instr.dst() as usize, result);
        }
    }

    // =========================================================================
    // SHL - Shift Left (32-bit)
    // =========================================================================

    /// SHL r/m32, 1
    pub fn shl_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1 << 1;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 & 0x80000000) != 0;
        let of = ((result ^ op1) & 0x80000000) != 0;
        self.update_flags_shl32(result, cf, of);
        Ok(())
    }

    /// SHL r/m32, CL
    pub fn shl_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1 << count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
        // Bochs computes OF unconditionally: cf ^ (result >> 31)
        let of = cf != ((result >> 31) != 0);
        self.update_flags_shl32(result, cf, of);
        Ok(())
    }

    /// SHL r/m32, imm8
    pub fn shl_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1 << count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
        // Bochs computes OF unconditionally: cf ^ (result >> 31)
        let of = cf != ((result >> 31) != 0);
        self.update_flags_shl32(result, cf, of);
        Ok(())
    }

    // =========================================================================
    // SHR - Shift Right Logical (32-bit)
    // =========================================================================

    /// SHR r/m32, 1
    pub fn shr_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1 >> 1;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 & 0x00000001) != 0;
        let of = (op1 & 0x80000000) != 0;
        self.update_flags_shr32(result, cf, of);
        Ok(())
    }

    /// SHR r/m32, CL
    pub fn shr_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1 >> count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        // Bochs computes OF unconditionally: ((result << 1) ^ result) >> 31
        let of = (((result << 1) ^ result) >> 31) != 0;
        self.update_flags_shr32(result, cf, of);
        Ok(())
    }

    /// SHR r/m32, imm8
    pub fn shr_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1 >> count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        // Bochs computes OF unconditionally: ((result << 1) ^ result) >> 31
        let of = (((result << 1) ^ result) >> 31) != 0;
        self.update_flags_shr32(result, cf, of);
        Ok(())
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (32-bit)
    // =========================================================================

    /// SAR r/m32, 1
    pub fn sar_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1_u, laddr) = self.shift_read32(instr)?;
        let op1 = op1_u as i32;
        let result = (op1 >> 1) as u32;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
        Ok(())
    }

    /// SAR r/m32, CL
    pub fn sar_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read32(instr)?;
        let op1 = op1_u as i32;

        let result = (op1 >> count) as u32;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
        Ok(())
    }

    /// SAR r/m32, imm8
    pub fn sar_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x1F;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read32(instr)?;
        let op1 = op1_u as i32;

        let result = (op1 >> count) as u32;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
        Ok(())
    }

    // =========================================================================
    // ROL - Rotate Left (32-bit)
    // =========================================================================

    /// ROL r/m32, 1
    pub fn rol_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1.rotate_left(1);
        self.shift_write32(instr, laddr, result);

        let cf = result & 1 != 0;
        let of = ((result ^ (result >> 31)) & 1) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m32, CL
    pub fn rol_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1.rotate_left(count as u32);
        self.shift_write32(instr, laddr, result);

        let cf = result & 1 != 0;
        // Bochs computes OF unconditionally: bit0 ^ bit31
        let of = ((result ^ (result >> 31)) & 1) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m32, imm8
    pub fn rol_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write32(instr, laddr, result);

        let cf = result & 1 != 0;
        // Bochs computes OF unconditionally: bit0 ^ bit31
        let of = ((result ^ (result >> 31)) & 1) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // ROR - Rotate Right (32-bit)
    // =========================================================================

    /// ROR r/m32, 1
    pub fn ror_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1.rotate_right(1);
        self.shift_write32(instr, laddr, result);

        let cf = (result & 0x80000000) != 0;
        let of = ((result ^ (result << 1)) & 0x80000000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m32, CL
    pub fn ror_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1.rotate_right(count as u32);
        self.shift_write32(instr, laddr, result);

        let cf = (result & 0x80000000) != 0;
        // Bochs computes OF unconditionally: bit30 ^ bit31
        let of = ((result ^ (result << 1)) & 0x80000000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m32, imm8
    pub fn ror_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write32(instr, laddr, result);

        let cf = (result & 0x80000000) != 0;
        // Bochs computes OF unconditionally: bit30 ^ bit31
        let of = ((result ^ (result << 1)) & 0x80000000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // RCL - Rotate through Carry Left (32-bit)
    // Matches Bochs shift32.cc RCL_EdR / RCL_EdM
    // =========================================================================

    /// RCL r/m32, 1
    pub fn rcl_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read32(instr)?;
        let temp_cf = self.get_cf() as u32;
        let result = (op1 << 1) | temp_cf;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 >> 31) & 1;
        let of = cf ^ (result >> 31);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m32, CL
    pub fn rcl_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let temp_cf = self.get_cf() as u32;
        let result = if count == 1 {
            (op1 << 1) | temp_cf
        } else {
            (op1 << count) | (temp_cf << (count - 1)) | (op1 >> (33 - count))
        };
        self.shift_write32(instr, laddr, result);

        let cf = (op1 >> (32 - count)) & 1;
        let of = cf ^ (result >> 31);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m32, imm8
    pub fn rcl_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let temp_cf = self.get_cf() as u32;
        let result = if count == 1 {
            (op1 << 1) | temp_cf
        } else {
            (op1 << count) | (temp_cf << (count - 1)) | (op1 >> (33 - count))
        };
        self.shift_write32(instr, laddr, result);

        let cf = (op1 >> (32 - count)) & 1;
        let of = cf ^ (result >> 31);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // RCR - Rotate through Carry Right (32-bit)
    // Matches Bochs shift32.cc RCR_EdR / RCR_EdM
    // =========================================================================

    /// RCR r/m32, 1
    pub fn rcr_ed_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read32(instr)?;
        let temp_cf = self.get_cf() as u32;
        let result = (op1 >> 1) | (temp_cf << 31);
        self.shift_write32(instr, laddr, result);

        let cf = op1 & 1;
        let of = ((result << 1) ^ result) >> 31;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m32, CL
    pub fn rcr_ed_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let temp_cf = self.get_cf() as u32;
        let result = if count == 1 {
            (op1 >> 1) | (temp_cf << 31)
        } else {
            (op1 >> count) | (temp_cf << (32 - count)) | (op1 << (33 - count))
        };
        self.shift_write32(instr, laddr, result);

        let cf = (op1 >> (count - 1)) & 1;
        let of = ((result << 1) ^ result) >> 31;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m32, imm8
    pub fn rcr_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1, laddr) = self.shift_read32(instr)?;
        let temp_cf = self.get_cf() as u32;
        let result = if count == 1 {
            (op1 >> 1) | (temp_cf << 31)
        } else {
            (op1 >> count) | (temp_cf << (32 - count)) | (op1 << (33 - count))
        };
        self.shift_write32(instr, laddr, result);

        let cf = (op1 >> (count - 1)) & 1;
        let of = ((result << 1) ^ result) >> 31;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // Flag update helpers (32-bit)
    // =========================================================================

    fn update_flags_shl32(&mut self, result: u32, cf: bool, of: bool) {
        self.update_flags_logic32(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shr32(&mut self, result: u32, cf: bool, of: bool) {
        self.update_flags_logic32(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_sar32(&mut self, result: u32, cf: bool) {
        self.update_flags_logic32(result);
        if cf {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        self.eflags.remove(EFlags::OF);
    }

    // =========================================================================
    // SHLD - Double Precision Shift Left
    // Based on Bochs 
    // =========================================================================

    /// SHLD r/m32, r32, imm8
    /// Opcode: 0x0F 0xA4
    pub fn shld_ed_gd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1_32, laddr) = self.shift_read32(instr)?;
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op1_32 << count) | (op2_32 >> (32 - count));
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_32 >> 31) != 0);
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// SHLD r/m32, r32, CL
    /// Opcode: 0x0F 0xA5
    pub fn shld_ed_gd_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1_32, laddr) = self.shift_read32(instr)?;
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op1_32 << count) | (op2_32 >> (32 - count));
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_32 >> 31) != 0);
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // SHRD - Double Precision Shift Right
    // Based on Bochs 
    // =========================================================================

    /// SHRD r/m32, r32, imm8
    /// Opcode: 0x0F 0xAC
    pub fn shrd_ed_gd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1_32, laddr) = self.shift_read32(instr)?;
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op2_32 << (32 - count)) | (op1_32 >> count);
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = (((result_32 << 1) ^ result_32) >> 31) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// SHRD r/m32, r32, CL
    /// Opcode: 0x0F 0xAD
    pub fn shrd_ed_gd_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 {
            self.shift32_count0_clear(instr);
            return Ok(());
        }

        let (op1_32, laddr) = self.shift_read32(instr)?;
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op2_32 << (32 - count)) | (op1_32 >> count);
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = (((result_32 << 1) ^ result_32) >> 31) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }
}
