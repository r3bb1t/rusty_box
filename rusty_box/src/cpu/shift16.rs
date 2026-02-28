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
        let count = self.cl() & 0x0F; // Only low 4 bits for 16-bit rotate
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_left(count as u32);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROL r/m16, imm8
    pub fn rol_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x0F; // Only low 4 bits for 16-bit rotate
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_left(count as u32);
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
        let count = self.cl() & 0x0F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_right(count as u32);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
        Ok(())
    }

    /// ROR r/m16, imm8
    pub fn ror_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let count = instr.ib() & 0x0F;
        if count == 0 {
            return Ok(());
        }

        let (op1, laddr) = self.shift_read16(instr)?;
        let result = op1.rotate_right(count as u32);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
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
