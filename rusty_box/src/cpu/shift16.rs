//! 16-bit shift and rotate instructions
//!
//! Based on Bochs shift16.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements SHL, SHR, SAR, ROL, ROR for 16-bit operands

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ---- 16-bit read/write helpers for shift instructions ----
    fn shift_read16(&mut self, instr: &Instruction) -> super::Result<(u16, Option<()>)> {
        if instr.mod_c0() {
            Ok((self.get_gpr16(instr.dst() as usize), None))
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.read_rmw_virtual_word(seg, eaddr)?;
            Ok((val, Some(())))
        }
    }
    fn shift_write16(&mut self, instr: &Instruction, paddr: Option<()>, result: u16) {
        if let Some(_) = paddr {
            self.write_rmw_linear_word(result);
        } else {
            self.set_gpr16(instr.dst() as usize, result);
        }
    }

    // =========================================================================
    // SHL - Shift Left (16-bit)
    // =========================================================================

    /// SHL r/m16, 1
    pub fn shl_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1 << 1;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 & 0x8000) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.update_flags_shl16(result, cf, of);
        Ok(())
    }

    /// SHL r/m16, CL
    pub fn shl_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = if count >= 16 { 0 } else { op1 << count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 {
            false
        } else {
            ((op1 << (count - 1)) & 0x8000) != 0
        };
        let of = if count == 1 {
            ((result ^ op1) & 0x8000) != 0
        } else {
            false
        };
        self.update_flags_shl16(result, cf, of);
        Ok(())
    }

    /// SHL r/m16, imm8
    pub fn shl_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = if count >= 16 { 0 } else { op1 << count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 {
            false
        } else {
            ((op1 << (count - 1)) & 0x8000) != 0
        };
        let of = if count == 1 {
            ((result ^ op1) & 0x8000) != 0
        } else {
            false
        };
        self.update_flags_shl16(result, cf, of);
        Ok(())
    }

    // =========================================================================
    // SHR - Shift Right Logical (16-bit)
    // =========================================================================

    /// SHR r/m16, 1
    pub fn shr_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1 >> 1;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 & 0x0001) != 0;
        let of = (op1 & 0x8000) != 0;
        self.update_flags_shr16(result, cf, of);
        Ok(())
    }

    /// SHR r/m16, CL
    pub fn shr_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = if count >= 16 { 0 } else { op1 >> count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 {
            false
        } else {
            ((op1 >> (count - 1)) & 0x0001) != 0
        };
        let of = if count == 1 {
            (op1 & 0x8000) != 0
        } else {
            false
        };
        self.update_flags_shr16(result, cf, of);
        Ok(())
    }

    /// SHR r/m16, imm8
    pub fn shr_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = if count >= 16 { 0 } else { op1 >> count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 {
            false
        } else {
            ((op1 >> (count - 1)) & 0x0001) != 0
        };
        let of = if count == 1 {
            (op1 & 0x8000) != 0
        } else {
            false
        };
        self.update_flags_shr16(result, cf, of);
        Ok(())
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (16-bit)
    // =========================================================================

    /// SAR r/m16, 1
    pub fn sar_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1_u, laddr) = self.shift_read16(instr)?;
        let op1 = op1_u as i16;
        let result = (op1 >> 1) as u16;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 & 0x0001) != 0;
        self.update_flags_sar16(result, cf);
        Ok(())
    }

    /// SAR r/m16, CL
    pub fn sar_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read16(instr)?;
        let op1 = op1_u as i16;

        let result = if count >= 16 {
            if op1 < 0 {
                0xFFFF
            } else {
                0
            }
        } else {
            (op1 >> count) as u16
        };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 {
            op1 < 0
        } else {
            (op1 >> (count - 1)) & 0x0001 != 0
        };
        self.update_flags_sar16(result, cf);
        Ok(())
    }

    /// SAR r/m16, imm8
    pub fn sar_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read16(instr)?;
        let op1 = op1_u as i16;

        let result = if count >= 16 {
            if op1 < 0 {
                0xFFFF
            } else {
                0
            }
        } else {
            (op1 >> count) as u16
        };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 {
            op1 < 0
        } else {
            (op1 >> (count - 1)) & 0x0001 != 0
        };
        self.update_flags_sar16(result, cf);
        Ok(())
    }

    // =========================================================================
    // ROL - Rotate Left (16-bit)
    // =========================================================================

    /// ROL r/m16, 1
    pub fn rol_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_left(1);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m16, CL
    pub fn rol_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs shift16.cc: masks CL to 5 bits (0..31), then uses low 4 bits as actual rotate count
        let cl = self.cl() & 0x1F;
        let count = (cl & 0x0F) as u32;
        if count == 0 {
            // count & 0x0F == 0: no rotation. But if bit 4 is set (cl=16), still update CF/OF.
            // Bochs shift16.cc:234-241
            if cl & 0x10 != 0 {
                let (op1, _) = self.shift_read16(instr)?;
                let bit0 = (op1 & 0x0001) != 0;
                let bit15 = (op1 >> 15) != 0;
                self.set_cf_of(bit0, bit0 != bit15);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m16, imm8
    pub fn rol_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs: masks imm8 to 5 bits, uses low 4 bits as actual rotate count
        let imm = instr.ib() & 0x1F;
        let count = (imm & 0x0F) as u32;
        if count == 0 {
            // If bit 4 is set, update CF/OF without rotating. Bochs shift16.cc:267-272
            if imm & 0x10 != 0 {
                let (op1, _) = self.shift_read16(instr)?;
                let bit0 = (op1 & 0x0001) != 0;
                let bit15 = (op1 >> 15) != 0;
                self.set_cf_of(bit0, bit0 != bit15);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // ROR - Rotate Right (16-bit)
    // =========================================================================

    /// ROR r/m16, 1
    pub fn ror_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_right(1);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m16, CL
    pub fn ror_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs shift16.cc: masks CL to 5 bits, uses low 4 bits as actual rotate count
        let cl = self.cl() & 0x1F;
        let count = (cl & 0x0F) as u32;
        if count == 0 {
            // If bit 4 is set (cl=16), update CF/OF without rotating. Bochs shift16.cc:292-303
            if cl & 0x10 != 0 {
                let (op1, _) = self.shift_read16(instr)?;
                let bit15 = (op1 >> 15) != 0;
                let bit14 = ((op1 >> 14) & 1) != 0;
                self.set_cf_of(bit15, bit15 != bit14);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m16, imm8
    pub fn ror_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs: masks imm8 to 5 bits, uses low 4 bits as actual rotate count
        let imm = instr.ib() & 0x1F;
        let count = (imm & 0x0F) as u32;
        if count == 0 {
            // If bit 4 is set, update CF/OF without rotating. Bochs shift16.cc:314-325
            if imm & 0x10 != 0 {
                let (op1, _) = self.shift_read16(instr)?;
                let bit15 = (op1 >> 15) != 0;
                let bit14 = ((op1 >> 14) & 1) != 0;
                self.set_cf_of(bit15, bit15 != bit14);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // RCL - Rotate through Carry Left (16-bit)
    // Matches Bochs shift16.cc RCL_EwR / RCL_EwM
    // =========================================================================

    /// RCL r/m16, 1
    pub fn rcl_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read16(instr)?;
        let temp_cf = self.get_cf() as u16;
        let result = (op1 << 1) | temp_cf;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 >> 15) & 1;
        let of = cf ^ (result >> 15);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m16, CL
    pub fn rcl_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((self.cl() & 0x1F) % 17) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result = if count == 1 {
            ((op1_32 << 1) | temp_cf) as u16
        } else {
            ((op1_32 << count) | (temp_cf << (count - 1)) | (op1_32 >> (17 - count))) as u16
        };
        self.shift_write16(instr, laddr, result);

        let cf = (op1_32 >> (16 - count)) & 1;
        let of = cf ^ ((result >> 15) as u32);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m16, imm8
    pub fn rcl_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((instr.ib() & 0x1F) % 17) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result = if count == 1 {
            ((op1_32 << 1) | temp_cf) as u16
        } else {
            ((op1_32 << count) | (temp_cf << (count - 1)) | (op1_32 >> (17 - count))) as u16
        };
        self.shift_write16(instr, laddr, result);

        let cf = (op1_32 >> (16 - count)) & 1;
        let of = cf ^ ((result >> 15) as u32);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // RCR - Rotate through Carry Right (16-bit)
    // Matches Bochs shift16.cc RCR_EwR / RCR_EwM
    // =========================================================================

    /// RCR r/m16, 1
    pub fn rcr_ew_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read16(instr)?;
        let temp_cf = self.get_cf() as u16;
        let result = (op1 >> 1) | (temp_cf << 15);
        self.shift_write16(instr, laddr, result);

        let cf = op1 & 1;
        let of = ((result << 1) ^ result) >> 15;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m16, CL
    pub fn rcr_ew_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((self.cl() & 0x1F) % 17) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result =
            ((op1_32 >> count) | (temp_cf << (16 - count)) | (op1_32 << (17 - count))) as u16;
        self.shift_write16(instr, laddr, result);

        let cf = (op1_32 >> (count - 1)) & 1;
        let of = ((((result as u32) << 1) ^ (result as u32)) >> 15) & 1;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m16, imm8
    pub fn rcr_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((instr.ib() & 0x1F) % 17) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result =
            ((op1_32 >> count) | (temp_cf << (16 - count)) | (op1_32 << (17 - count))) as u16;
        self.shift_write16(instr, laddr, result);

        let cf = (op1_32 >> (count - 1)) & 1;
        let of = ((((result as u32) << 1) ^ (result as u32)) >> 15) & 1;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // SHLD - Double Precision Shift Left (16-bit)
    // Based on Bochs shift16.cc:30-123
    // =========================================================================

    /// SHLD r/m16, r16, imm8
    /// Opcode: 0x0F 0xA4
    pub fn shld_ew_gw_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1_16, laddr) = self.shift_read16(instr)?;
        let op2_16 = self.get_gpr16(instr.src() as usize) as u32;
        let op1_32 = op1_16 as u32;

        // count < 32, since only lower 5 bits used
        let temp_32 = (op1_32 << 16) | op2_16; // double formed by op1:op2
        let mut result_32 = temp_32 << count;

        // hack to act like x86 SHLD when count > 16
        // P6 way: shifting op2:op1 by count-16
        if count > 16 {
            result_32 |= op1_32 << (count - 16);
        }

        let result_16 = (result_32 >> 16) as u16;

        self.shift_write16(instr, laddr, result_16);

        self.update_flags_logic16(result_16);
        let cf = ((temp_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_16 >> 15) != 0); // of = cf ^ result15
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// SHLD r/m16, r16, CL
    /// Opcode: 0x0F 0xA5
    pub fn shld_ew_gw_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1_16, laddr) = self.shift_read16(instr)?;
        let op2_16 = self.get_gpr16(instr.src() as usize) as u32;
        let op1_32 = op1_16 as u32;

        // count < 32, since only lower 5 bits used
        let temp_32 = (op1_32 << 16) | op2_16; // double formed by op1:op2
        let mut result_32 = temp_32 << count;

        // hack to act like x86 SHLD when count > 16
        // P6 way: shifting op2:op1 by count-16
        if count > 16 {
            result_32 |= op1_32 << (count - 16);
        }

        let result_16 = (result_32 >> 16) as u16;

        self.shift_write16(instr, laddr, result_16);

        self.update_flags_logic16(result_16);
        let cf = ((temp_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_16 >> 15) != 0); // of = cf ^ result15
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // SHRD - Double Precision Shift Right (16-bit)
    // Based on Bochs shift16.cc:125-218
    // =========================================================================

    /// SHRD r/m16, r16, imm8
    /// Opcode: 0x0F 0xAC
    pub fn shrd_ew_gw_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1_16, laddr) = self.shift_read16(instr)?;
        let op2_16 = self.get_gpr16(instr.src() as usize) as u32;
        let op1_32 = op1_16 as u32;

        // count < 32, since only lower 5 bits used
        let temp_32 = (op2_16 << 16) | op1_32; // double formed by op2:op1
        let mut result_32 = temp_32 >> count;

        // hack to act like x86 SHRD when count > 16
        // P6 way: shifting op1:op2 by count-16
        if count > 16 {
            result_32 |= op1_32 << (32 - count);
        }

        let result_16 = result_32 as u16;

        self.shift_write16(instr, laddr, result_16);

        self.update_flags_logic16(result_16);
        let mut cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = ((((result_16 as u32) << 1) ^ (result_16 as u32)) >> 15) & 0x1 != 0; // of = result14 ^ result15
        if count > 16 {
            cf = ((op2_16 >> (count - 17)) & 0x1) != 0; // undefined flags behavior matching real HW
        }
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// SHRD r/m16, r16, CL
    /// Opcode: 0x0F 0xAD
    pub fn shrd_ew_gw_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1_16, laddr) = self.shift_read16(instr)?;
        let op2_16 = self.get_gpr16(instr.src() as usize) as u32;
        let op1_32 = op1_16 as u32;

        // count < 32, since only lower 5 bits used
        let temp_32 = (op2_16 << 16) | op1_32; // double formed by op2:op1
        let mut result_32 = temp_32 >> count;

        // hack to act like x86 SHRD when count > 16
        // P6 way: shifting op1:op2 by count-16
        if count > 16 {
            result_32 |= op1_32 << (32 - count);
        }

        let result_16 = result_32 as u16;

        self.shift_write16(instr, laddr, result_16);

        self.update_flags_logic16(result_16);
        let mut cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = ((((result_16 as u32) << 1) ^ (result_16 as u32)) >> 15) & 0x1 != 0; // of = result14 ^ result15
        if count > 16 {
            cf = ((op2_16 >> (count - 17)) & 0x1) != 0; // undefined flags behavior matching real HW
        }
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // Flag update helpers (16-bit)
    // =========================================================================

    fn update_flags_shl16(&mut self, result: u16, cf: bool, of: bool) {
        self.update_flags_logic16(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shr16(&mut self, result: u16, cf: bool, of: bool) {
        self.update_flags_logic16(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_sar16(&mut self, result: u16, cf: bool) {
        self.update_flags_logic16(result);
        if cf {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        self.eflags.remove(EFlags::OF);
    }
}
