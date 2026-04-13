//! 8-bit shift and rotate instructions
//!
//! Based on Bochs shift8.cc
//!
//! Implements SHL, SHR, SAR, ROL, ROR for 8-bit operands

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ---- 8-bit read/write helpers for shift instructions ----
    fn shift_read8(&mut self, instr: &Instruction) -> super::Result<(u8, Option<()>)> {
        if instr.mod_c0() {
            Ok((self.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()), None))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_rmw_byte(seg, eaddr)?;
            Ok((val, Some(())))
        }
    }
    fn shift_write8(&mut self, instr: &Instruction, paddr: Option<()>, result: u8) {
        if paddr.is_some() {
            self.write_rmw_linear_byte(result);
        } else {
            self.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), result);
        }
    }

    // =========================================================================
    // SHL - Shift Left (8-bit)
    // =========================================================================

    /// SHL r/m8, 1
    pub fn shl_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1 << 1;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 & 0x80) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.update_flags_shl8(result, cf, of);
        Ok(())
    }

    /// SHL r/m8, CL
    pub fn shl_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = if count >= 8 { 0 } else { op1 << count };
        self.shift_write8(instr, laddr, result);

        // Bochs shift8.cc: count <= 8 computes CF from formula, count > 8 CF=0
        let cf = if count > 8 {
            false
        } else {
            ((op1 << (count - 1)) & 0x80) != 0
        };
        // Bochs computes OF unconditionally: cf ^ (result >> 7)
        let of = cf != ((result >> 7) != 0);
        self.update_flags_shl8(result, cf, of);
        Ok(())
    }

    /// SHL r/m8, imm8
    pub fn shl_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = if count >= 8 { 0 } else { op1 << count };
        self.shift_write8(instr, laddr, result);

        // Bochs shift8.cc: count <= 8 computes CF from formula, count > 8 CF=0
        let cf = if count > 8 {
            false
        } else {
            ((op1 << (count - 1)) & 0x80) != 0
        };
        // Bochs computes OF unconditionally: cf ^ (result >> 7)
        let of = cf != ((result >> 7) != 0);
        self.update_flags_shl8(result, cf, of);
        Ok(())
    }

    // =========================================================================
    // SHR - Shift Right Logical (8-bit)
    // =========================================================================

    /// SHR r/m8, 1
    pub fn shr_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1 >> 1;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 & 0x01) != 0;
        let of = (op1 & 0x80) != 0;
        self.update_flags_shr8(result, cf, of);
        Ok(())
    }

    /// SHR r/m8, CL
    pub fn shr_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = if count >= 8 { 0 } else { op1 >> count };
        self.shift_write8(instr, laddr, result);

        // Bochs shift8.cc: no boundary check for SHR, formula works for all counts 1-31
        // Cast to u32 to avoid Rust panic on shift >= 8 for u8
        let cf = (((op1 as u32) >> (count - 1)) & 0x01) != 0;
        // Bochs computes OF unconditionally: ((result << 1) ^ result) >> 7
        let of = ((((result as u32) << 1) ^ (result as u32)) >> 7) != 0;
        self.update_flags_shr8(result, cf, of);
        Ok(())
    }

    /// SHR r/m8, imm8
    pub fn shr_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = if count >= 8 { 0 } else { op1 >> count };
        self.shift_write8(instr, laddr, result);

        // Bochs shift8.cc: no boundary check for SHR, formula works for all counts 1-31
        // Cast to u32 to avoid Rust panic on shift >= 8 for u8
        let cf = (((op1 as u32) >> (count - 1)) & 0x01) != 0;
        // Bochs computes OF unconditionally: ((result << 1) ^ result) >> 7
        let of = ((((result as u32) << 1) ^ (result as u32)) >> 7) != 0;
        self.update_flags_shr8(result, cf, of);
        Ok(())
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (8-bit)
    // =========================================================================

    /// SAR r/m8, 1
    pub fn sar_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1_u, laddr) = self.shift_read8(instr)?;
        let op1 = op1_u as i8;
        let result = (op1 >> 1) as u8;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 & 0x01) != 0;
        self.update_flags_sar8(result, cf);
        Ok(())
    }

    /// SAR r/m8, CL
    pub fn sar_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = self.cl() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1_u, laddr) = self.shift_read8(instr)?;
        let op1 = op1_u as i8;

        let result = if count >= 8 {
            if op1 < 0 {
                0xFF
            } else {
                0
            }
        } else {
            (op1 >> count) as u8
        };
        self.shift_write8(instr, laddr, result);

        let cf = if count >= 8 {
            op1 < 0
        } else {
            (op1 >> (count - 1)) & 0x01 != 0
        };
        self.update_flags_sar8(result, cf);
        Ok(())
    }

    /// SAR r/m8, imm8
    pub fn sar_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x1F;
        if count == 0 {
            return Ok(());
        }

        let (op1_8, laddr) = self.shift_read8(instr)?;
        let result_8 = ((op1_8 as i8) >> count) as u8;
        self.shift_write8(instr, laddr, result_8);

        let cf = (((op1_8 as i8) >> (count - 1)) & 0x1) != 0;
        self.update_flags_logic8(result_8);
        self.set_cf_of(cf, false);
        Ok(())
    }

    // =========================================================================
    // ROL - Rotate Left (8-bit)
    // =========================================================================

    /// ROL r/m8, 1
    pub fn rol_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1.rotate_left(1);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m8, CL
    pub fn rol_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs shift8.cc: uses raw CL, checks count & 0x07 and count & 0x18
        let cl = self.cl();
        let count = (cl & 0x07) as u32;
        if count == 0 {
            // count & 0x07 == 0: no rotation. If count & 0x18 != 0 (cl=8,16,24), update flags.
            // Bochs shift8.cc
            if cl & 0x18 != 0 {
                let (op1, _) = self.shift_read8(instr)?;
                let bit0 = (op1 & 0x01) != 0;
                let bit7 = (op1 >> 7) != 0;
                self.set_cf_of(bit0, bit0 != bit7);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m8, imm8
    pub fn rol_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs shift8.cc: uses raw imm8, checks count & 0x07 and count & 0x18
        let imm = instr.ib();
        let count = (imm & 0x07) as u32;
        if count == 0 {
            // If bit 3 or 4 set (imm=8,16,24), update flags. Bochs shift8.cc
            if imm & 0x18 != 0 {
                let (op1, _) = self.shift_read8(instr)?;
                let bit0 = (op1 & 0x01) != 0;
                let bit7 = (op1 >> 7) != 0;
                self.set_cf_of(bit0, bit0 != bit7);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1.rotate_left(count);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // ROR - Rotate Right (8-bit)
    // =========================================================================

    /// ROR r/m8, 1
    pub fn ror_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1.rotate_right(1);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m8, CL
    pub fn ror_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs shift8.cc: uses raw CL, checks count & 0x07 and count & 0x18
        let cl = self.cl();
        let count = (cl & 0x07) as u32;
        if count == 0 {
            // If count & 0x18 != 0 (cl=8,16,24), update CF/OF without rotating.
            // Bochs shift8.cc: CF=bit7, OF=bit6^bit7
            if cl & 0x18 != 0 {
                let (op1, _) = self.shift_read8(instr)?;
                let bit6 = ((op1 >> 6) & 1) != 0;
                let bit7 = (op1 >> 7) != 0;
                self.set_cf_of(bit7, bit6 != bit7);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m8, imm8
    pub fn ror_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs shift8.cc: uses raw imm8, checks count & 0x07 and count & 0x18
        let imm = instr.ib();
        let count = (imm & 0x07) as u32;
        if count == 0 {
            // If imm & 0x18 != 0 (imm=8,16,24), update CF/OF without rotating.
            // Bochs shift8.cc: CF=bit7, OF=bit6^bit7
            if imm & 0x18 != 0 {
                let (op1, _) = self.shift_read8(instr)?;
                let bit6 = ((op1 >> 6) & 1) != 0;
                let bit7 = (op1 >> 7) != 0;
                self.set_cf_of(bit7, bit6 != bit7);
            }
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let result = op1.rotate_right(count);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    // =========================================================================
    // RCL - Rotate through Carry Left (8-bit)
    // Matches Bochs shift8.cc RCL_EbR / RCL_EbM
    // =========================================================================

    /// RCL r/m8, 1
    pub fn rcl_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read8(instr)?;
        let temp_cf = self.get_cf() as u8;
        let result = (op1 << 1) | temp_cf;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 >> 7) & 1;
        let of = cf ^ (result >> 7);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m8, CL
    pub fn rcl_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((self.cl() & 0x1F) % 9) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result = if count == 1 {
            ((op1_32 << 1) | temp_cf) as u8
        } else {
            ((op1_32 << count) | (temp_cf << (count - 1)) | (op1_32 >> (9 - count))) as u8
        };
        self.shift_write8(instr, laddr, result);

        let cf = (op1_32 >> (8 - count)) & 1;
        let of = cf ^ ((result >> 7) as u32);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCL r/m8, imm8
    pub fn rcl_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((instr.ib() & 0x1F) % 9) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result = if count == 1 {
            ((op1_32 << 1) | temp_cf) as u8
        } else {
            ((op1_32 << count) | (temp_cf << (count - 1)) | (op1_32 >> (9 - count))) as u8
        };
        self.shift_write8(instr, laddr, result);

        let cf = (op1_32 >> (8 - count)) & 1;
        let of = cf ^ ((result >> 7) as u32);
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // RCR - Rotate through Carry Right (8-bit)
    // Matches Bochs shift8.cc RCR_EbR / RCR_EbM
    // =========================================================================

    /// RCR r/m8, 1
    pub fn rcr_eb_1(&mut self, instr: &Instruction) -> super::Result<()> {
        let (op1, laddr) = self.shift_read8(instr)?;
        let temp_cf = self.get_cf() as u8;
        let result = (op1 >> 1) | (temp_cf << 7);
        self.shift_write8(instr, laddr, result);

        let cf = op1 & 1;
        let of = (((result << 1) ^ result) >> 7) & 1;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m8, CL
    pub fn rcr_eb_cl(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((self.cl() & 0x1F) % 9) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result = ((op1_32 >> count) | (temp_cf << (8 - count)) | (op1_32 << (9 - count))) as u8;
        self.shift_write8(instr, laddr, result);

        let cf = (op1_32 >> (count - 1)) & 1;
        let of = ((((result as u32) << 1) ^ (result as u32)) >> 7) & 1;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    /// RCR r/m8, imm8
    pub fn rcr_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = ((instr.ib() & 0x1F) % 9) as u32;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read8(instr)?;
        let temp_cf = self.get_cf() as u32;
        let op1_32 = op1 as u32;
        let result = ((op1_32 >> count) | (temp_cf << (8 - count)) | (op1_32 << (9 - count))) as u8;
        self.shift_write8(instr, laddr, result);

        let cf = (op1_32 >> (count - 1)) & 1;
        let of = ((((result as u32) << 1) ^ (result as u32)) >> 7) & 1;
        self.set_cf_of(cf != 0, of != 0);
        Ok(())
    }

    // =========================================================================
    // Flag update helpers (8-bit)
    // =========================================================================

    fn update_flags_shl8(&mut self, result: u8, cf: bool, of: bool) {
        self.update_flags_logic8(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shr8(&mut self, result: u8, cf: bool, of: bool) {
        self.update_flags_logic8(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_sar8(&mut self, result: u8, cf: bool) {
        self.update_flags_logic8(result);
        if cf {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        // OF is always 0 for SAR by 1
        self.eflags.remove(EFlags::OF);
    }

    // =========================================================================
    // Shared helper: set CF and OF flags
    // Used by shift8, shift16, and shift32
    // =========================================================================

    pub(super) fn set_cf_of(&mut self, cf: bool, of: bool) {
        if cf {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        } else {
            self.eflags.remove(EFlags::OF);
        }
    }
}
