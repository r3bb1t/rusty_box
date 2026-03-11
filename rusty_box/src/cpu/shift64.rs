//! 64-bit shift and rotate instructions
//!
//! Based on Bochs shift64.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements SHL, SHR, SAR, ROL, ROR, RCL, RCR, SHLD, SHRD for 64-bit operands

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ---- 64-bit read/write helpers for shift instructions ----
    fn shift_read64(&mut self, instr: &Instruction) -> super::Result<(u64, Option<u64>)> {
        if instr.mod_c0() {
            Ok((self.get_gpr64(instr.dst() as usize), None))
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            let seg_idx = seg as usize;
            let laddr = self.get_laddr64(seg_idx, eaddr);
            let (val, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
            Ok((val, Some(rmw_laddr)))
        }
    }

    fn shift_write64(&mut self, instr: &Instruction, paddr: Option<u64>, result: u64) {
        if let Some(laddr) = paddr {
            self.write_rmw_linear_qword(laddr, result);
        } else {
            self.set_gpr64(instr.dst() as usize, result);
        }
    }

    // =========================================================================
    // SHL - Shift Left (64-bit)
    // Based on Bochs shift64.cc SHL_EqM / SHL_EqR
    // =========================================================================

    /// SHL r/m64, 1 or CL or imm8
    /// Unified handler — caller passes count via instr.ib() or CL dispatch
    pub fn shl_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1 << 1;
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> 63) & 1;
        let of = cf ^ (result >> 63);
        self.set_flags_oszapc_logic_64(result);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// SHL r/m64, CL
    pub fn shl_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1 << count;
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (64 - count)) & 0x1;
        let of = cf ^ (result >> 63);
        self.set_flags_oszapc_logic_64(result);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// SHL r/m64, imm8
    pub fn shl_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1 << count;
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (64 - count)) & 0x1;
        let of = cf ^ (result >> 63);
        self.set_flags_oszapc_logic_64(result);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // SHR - Shift Right Logical (64-bit)
    // Based on Bochs shift64.cc SHR_EqM / SHR_EqR
    // =========================================================================

    /// SHR r/m64, 1
    pub fn shr_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1 >> 1;
        self.shift_write64(instr, laddr, result);

        let cf = op1 & 0x1;
        // of = result62 ^ result63 (which equals op1_63 ^ 0 = op1_63 for count==1)
        let of = ((result << 1) ^ result) >> 63;
        self.set_flags_oszapc_logic_64(result);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// SHR r/m64, CL
    pub fn shr_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1 >> count;
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (count - 1)) & 0x1;
        // of == result63 if count==1, else 0
        let of = ((result << 1) ^ result) >> 63;
        self.set_flags_oszapc_logic_64(result);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// SHR r/m64, imm8
    pub fn shr_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1 >> count;
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (count - 1)) & 0x1;
        // of == result63 if count==1, else 0
        let of = ((result << 1) ^ result) >> 63;
        self.set_flags_oszapc_logic_64(result);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (64-bit)
    // Based on Bochs shift64.cc SAR_EqM / SAR_EqR
    // =========================================================================

    /// SAR r/m64, 1
    pub fn sar_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1_u, laddr) = self.shift_read64(instr)?;
        let op1 = op1_u as i64;
        let result = (op1 >> 1) as u64;
        self.shift_write64(instr, laddr, result);

        let cf = op1_u & 0x1;
        self.set_flags_oszapc_logic_64(result);
        // signed overflow cannot happen in SAR
        self.set_cf_of(cf != 0, false);
        Ok(())
    }

    /// SAR r/m64, CL
    pub fn sar_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read64(instr)?;
        let op1 = op1_u as i64;
        let result = (op1 >> count) as u64;
        self.shift_write64(instr, laddr, result);

        let cf = (op1_u >> (count - 1)) & 0x1;
        self.set_flags_oszapc_logic_64(result);
        // signed overflow cannot happen in SAR
        self.set_cf_of(cf != 0, false);
        Ok(())
    }

    /// SAR r/m64, imm8
    pub fn sar_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read64(instr)?;
        let op1 = op1_u as i64;
        let result = (op1 >> count) as u64;
        self.shift_write64(instr, laddr, result);

        let cf = (op1_u >> (count - 1)) & 0x1;
        self.set_flags_oszapc_logic_64(result);
        // signed overflow cannot happen in SAR
        self.set_cf_of(cf != 0, false);
        Ok(())
    }

    // =========================================================================
    // ROL - Rotate Left (64-bit)
    // Based on Bochs shift64.cc ROL_EqM / ROL_EqR
    // =========================================================================

    /// ROL r/m64, 1
    pub fn rol_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1.rotate_left(1);
        self.shift_write64(instr, laddr, result);

        let bit0 = result & 0x1;
        let bit63 = result >> 63;
        // of = cf ^ result63
        self.set_cf_of(bit0 != 0, (bit0 ^ bit63) != 0);
        Ok(())
    }

    /// ROL r/m64, CL (or imm8 — unified in Bochs ROL_EqM/ROL_EqR)
    pub fn rol_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write64(instr, laddr, result);

        let bit0 = result & 0x1;
        let bit63 = result >> 63;
        // Bochs sets OF unconditionally (Intel says undefined for count>1)
        self.set_cf_of(bit0 != 0, (bit0 ^ bit63) != 0);
        Ok(())
    }

    /// ROL r/m64, imm8
    pub fn rol_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write64(instr, laddr, result);

        let bit0 = result & 0x1;
        let bit63 = result >> 63;
        // Bochs sets OF unconditionally (Intel says undefined for count>1)
        self.set_cf_of(bit0 != 0, (bit0 ^ bit63) != 0);
        Ok(())
    }

    // =========================================================================
    // ROR - Rotate Right (64-bit)
    // Based on Bochs shift64.cc ROR_EqM / ROR_EqR
    // =========================================================================

    /// ROR r/m64, 1
    pub fn ror_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1.rotate_right(1);
        self.shift_write64(instr, laddr, result);

        let bit63 = (result >> 63) & 1;
        let bit62 = (result >> 62) & 1;
        // of = result62 ^ result63
        self.set_cf_of(bit63 != 0, (bit62 ^ bit63) != 0);
        Ok(())
    }

    /// ROR r/m64, CL
    pub fn ror_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write64(instr, laddr, result);

        let bit63 = (result >> 63) & 1;
        let bit62 = (result >> 62) & 1;
        // Bochs computes OF unconditionally (Intel says undefined for count>1)
        self.set_cf_of(bit63 != 0, (bit62 ^ bit63) != 0);
        Ok(())
    }

    /// ROR r/m64, imm8
    pub fn ror_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write64(instr, laddr, result);

        let bit63 = (result >> 63) & 1;
        let bit62 = (result >> 62) & 1;
        // Bochs computes OF unconditionally (Intel says undefined for count>1)
        self.set_cf_of(bit63 != 0, (bit62 ^ bit63) != 0);
        Ok(())
    }

    // =========================================================================
    // RCL - Rotate through Carry Left (64-bit)
    // Based on Bochs shift64.cc RCL_EqM / RCL_EqR
    // =========================================================================

    /// RCL r/m64, 1
    pub fn rcl_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read64(instr)?;
        let temp_cf = self.get_cf() as u64;
        let result = (op1 << 1) | temp_cf;
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> 63) & 0x1;
        let of = cf ^ (result >> 63); // of = cf ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m64, CL
    pub fn rcl_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let temp_cf = self.get_cf() as u64;
        let result = if count == 1 {
            (op1 << 1) | temp_cf
        } else {
            (op1 << count) | (temp_cf << (count - 1)) | (op1 >> (65 - count))
        };
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (64 - count)) & 0x1;
        let of = cf ^ (result >> 63); // of = cf ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m64, imm8
    pub fn rcl_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let temp_cf = self.get_cf() as u64;
        let result = if count == 1 {
            (op1 << 1) | temp_cf
        } else {
            (op1 << count) | (temp_cf << (count - 1)) | (op1 >> (65 - count))
        };
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (64 - count)) & 0x1;
        let of = cf ^ (result >> 63); // of = cf ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // RCR - Rotate through Carry Right (64-bit)
    // Based on Bochs shift64.cc RCR_EqM / RCR_EqR
    // =========================================================================

    /// RCR r/m64, 1
    pub fn rcr_eq_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read64(instr)?;
        let temp_cf = self.get_cf() as u64;
        let result = (op1 >> 1) | (temp_cf << 63);
        self.shift_write64(instr, laddr, result);

        let cf = op1 & 0x1;
        let of = ((result << 1) ^ result) >> 63;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m64, CL
    pub fn rcr_eq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let temp_cf = self.get_cf() as u64;
        let result = if count == 1 {
            (op1 >> 1) | (temp_cf << 63)
        } else {
            (op1 >> count) | (temp_cf << (64 - count)) | (op1 << (65 - count))
        };
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (count - 1)) & 0x1;
        let of = ((result << 1) ^ result) >> 63;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m64, imm8
    pub fn rcr_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read64(instr)?;
        let temp_cf = self.get_cf() as u64;
        let result = if count == 1 {
            (op1 >> 1) | (temp_cf << 63)
        } else {
            (op1 >> count) | (temp_cf << (64 - count)) | (op1 << (65 - count))
        };
        self.shift_write64(instr, laddr, result);

        let cf = (op1 >> (count - 1)) & 0x1;
        let of = ((result << 1) ^ result) >> 63;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // SHLD - Double Precision Shift Left (64-bit)
    // Based on Bochs shift64.cc SHLD_EqGqM / SHLD_EqGqR
    // =========================================================================

    /// SHLD r/m64, r64, imm8
    /// Opcode: REX.W 0x0F 0xA4
    pub fn shld_eq_gq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1_64, laddr) = self.shift_read64(instr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);

        let result_64 = (op1_64 << count) | (op2_64 >> (64 - count));
        self.shift_write64(instr, laddr, result_64);

        self.set_flags_oszapc_logic_64(result_64);
        let cf = (op1_64 >> (64 - count)) & 0x1;
        let of = cf ^ (result_64 >> 63); // of = cf ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// SHLD r/m64, r64, CL
    /// Opcode: REX.W 0x0F 0xA5
    pub fn shld_eq_gq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1_64, laddr) = self.shift_read64(instr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);

        let result_64 = (op1_64 << count) | (op2_64 >> (64 - count));
        self.shift_write64(instr, laddr, result_64);

        self.set_flags_oszapc_logic_64(result_64);
        let cf = (op1_64 >> (64 - count)) & 0x1;
        let of = cf ^ (result_64 >> 63); // of = cf ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // SHRD - Double Precision Shift Right (64-bit)
    // Based on Bochs shift64.cc SHRD_EqGqM / SHRD_EqGqR
    // =========================================================================

    /// SHRD r/m64, r64, imm8
    /// Opcode: REX.W 0x0F 0xAC
    pub fn shrd_eq_gq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (instr.ib() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1_64, laddr) = self.shift_read64(instr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);

        let result_64 = (op2_64 << (64 - count)) | (op1_64 >> count);
        self.shift_write64(instr, laddr, result_64);

        self.set_flags_oszapc_logic_64(result_64);
        let cf = (op1_64 >> (count - 1)) & 0x1;
        let of = ((result_64 << 1) ^ result_64) >> 63; // of = result62 ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// SHRD r/m64, r64, CL
    /// Opcode: REX.W 0x0F 0xAD
    pub fn shrd_eq_gq_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = (self.cl() & 0x3F) as u64;
        if count == 0 {
            return Ok(());
        }

        let (op1_64, laddr) = self.shift_read64(instr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);

        let result_64 = (op2_64 << (64 - count)) | (op1_64 >> count);
        self.shift_write64(instr, laddr, result_64);

        self.set_flags_oszapc_logic_64(result_64);
        let cf = (op1_64 >> (count - 1)) & 0x1;
        let of = ((result_64 << 1) ^ result_64) >> 63; // of = result62 ^ result63
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

}
