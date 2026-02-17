//! Shift and rotate instructions for x86 CPU emulation
//!
//! Based on Bochs shift8.cc, shift16.cc, shift32.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements SHL, SHR, SAR, ROL, ROR, RCL, RCR instructions

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // SHL - Shift Left (8-bit)
    // =========================================================================
    
    // ---- 8-bit read/write helpers for shift instructions ----
    fn shift_read8(&mut self, instr: &BxInstructionGenerated) -> (u8, Option<u32>) {
        if instr.mod_c0() {
            (self.get_gpr8(instr.dst() as usize), None)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let (val, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
            (val, Some(laddr))
        }
    }
    fn shift_write8(&mut self, instr: &BxInstructionGenerated, laddr: Option<u32>, result: u8) {
        if let Some(la) = laddr {
            self.write_rmw_linear_byte(la, result);
        } else {
            self.set_gpr8(instr.dst() as usize, result);
        }
    }
    // ---- 16-bit read/write helpers for shift instructions ----
    fn shift_read16(&mut self, instr: &BxInstructionGenerated) -> (u16, Option<u32>) {
        if instr.mod_c0() {
            (self.get_gpr16(instr.dst() as usize), None)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let (val, laddr) = self.read_rmw_virtual_word(seg, eaddr);
            (val, Some(laddr))
        }
    }
    fn shift_write16(&mut self, instr: &BxInstructionGenerated, laddr: Option<u32>, result: u16) {
        if let Some(la) = laddr {
            self.write_rmw_linear_word(la, result);
        } else {
            self.set_gpr16(instr.dst() as usize, result);
        }
    }
    // ---- 32-bit read/write helpers for shift instructions ----
    fn shift_read32(&mut self, instr: &BxInstructionGenerated) -> (u32, Option<u32>) {
        if instr.mod_c0() {
            (self.get_gpr32(instr.dst() as usize), None)
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let (val, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
            (val, Some(laddr))
        }
    }
    fn shift_write32(&mut self, instr: &BxInstructionGenerated, laddr: Option<u32>, result: u32) {
        if let Some(la) = laddr {
            self.write_rmw_linear_dword(la, result);
        } else {
            self.set_gpr32(instr.dst() as usize, result);
        }
    }

    /// SHL r/m8, 1
    pub fn shl_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read8(instr);
        let result = op1 << 1;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 & 0x80) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.update_flags_shl8(result, cf, of);
    }

    /// SHL r/m8, CL
    pub fn shl_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read8(instr);
        let result = if count >= 8 { 0 } else { op1 << count };
        self.shift_write8(instr, laddr, result);

        let cf = if count >= 8 { false } else { ((op1 << (count - 1)) & 0x80) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x80) != 0 } else { false };
        self.update_flags_shl8(result, cf, of);
    }

    /// SHL r/m8, imm8
    pub fn shl_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = instr.ib() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read8(instr);
        let result = if count >= 8 { 0 } else { op1 << count };
        self.shift_write8(instr, laddr, result);

        let cf = if count >= 8 { false } else { ((op1 << (count - 1)) & 0x80) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x80) != 0 } else { false };
        self.update_flags_shl8(result, cf, of);
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (8-bit)
    // =========================================================================
    
    /// SAR r/m8, imm8
    pub fn sar_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = instr.ib() & 0x1F;
        if count == 0 { return; }

        let (op1_8, laddr) = self.shift_read8(instr);
        let result_8 = ((op1_8 as i8) >> count) as u8;
        self.shift_write8(instr, laddr, result_8);

        let cf = (((op1_8 as i8) >> (count - 1)) & 0x1) != 0;
        self.update_flags_logic8(result_8);
        self.set_cf_of(cf, false);
    }

    // =========================================================================
    // SHL - Shift Left (16-bit)
    // =========================================================================
    
    /// SHL r/m16, 1
    pub fn shl_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read16(instr);
        let result = op1 << 1;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 & 0x8000) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.update_flags_shl16(result, cf, of);
    }

    /// SHL r/m16, CL
    pub fn shl_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read16(instr);
        let result = if count >= 16 { 0 } else { op1 << count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 { false } else { ((op1 << (count - 1)) & 0x8000) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x8000) != 0 } else { false };
        self.update_flags_shl16(result, cf, of);
    }

    /// SHL r/m16, imm8
    pub fn shl_ew_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read16(instr);
        let result = if count >= 16 { 0 } else { op1 << count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 { false } else { ((op1 << (count - 1)) & 0x8000) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x8000) != 0 } else { false };
        self.update_flags_shl16(result, cf, of);
    }

    // =========================================================================
    // SHL - Shift Left (32-bit)
    // =========================================================================
    
    /// SHL r/m32, 1
    pub fn shl_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read32(instr);
        let result = op1 << 1;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 & 0x80000000) != 0;
        let of = ((result ^ op1) & 0x80000000) != 0;
        self.update_flags_shl32(result, cf, of);
    }

    /// SHL r/m32, CL
    pub fn shl_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1 << count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
        let of = if count == 1 { ((result ^ op1) & 0x80000000) != 0 } else { false };
        self.update_flags_shl32(result, cf, of);
    }

    /// SHL r/m32, imm8
    pub fn shl_ed_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1 << count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
        let of = if count == 1 { ((result ^ op1) & 0x80000000) != 0 } else { false };
        self.update_flags_shl32(result, cf, of);
    }

    // =========================================================================
    // SHR - Shift Right Logical (8-bit)
    // =========================================================================
    
    /// SHR r/m8, 1
    pub fn shr_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read8(instr);
        let result = op1 >> 1;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 & 0x01) != 0;
        let of = (op1 & 0x80) != 0;
        self.update_flags_shr8(result, cf, of);
    }

    /// SHR r/m8, CL
    pub fn shr_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read8(instr);
        let result = if count >= 8 { 0 } else { op1 >> count };
        self.shift_write8(instr, laddr, result);

        let cf = if count >= 8 { false } else { ((op1 >> (count - 1)) & 0x01) != 0 };
        let of = if count == 1 { (op1 & 0x80) != 0 } else { false };
        self.update_flags_shr8(result, cf, of);
    }

    /// SHR r/m8, imm8
    pub fn shr_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = instr.ib() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read8(instr);
        let result = if count >= 8 { 0 } else { op1 >> count };
        self.shift_write8(instr, laddr, result);

        let cf = if count >= 8 { false } else { ((op1 >> (count - 1)) & 0x01) != 0 };
        let of = if count == 1 { (op1 & 0x80) != 0 } else { false };
        self.update_flags_shr8(result, cf, of);
    }

    // =========================================================================
    // SHR - Shift Right Logical (16-bit)
    // =========================================================================
    
    /// SHR r/m16, 1
    pub fn shr_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read16(instr);
        let result = op1 >> 1;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 & 0x0001) != 0;
        let of = (op1 & 0x8000) != 0;
        self.update_flags_shr16(result, cf, of);
    }

    /// SHR r/m16, CL
    pub fn shr_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read16(instr);
        let result = if count >= 16 { 0 } else { op1 >> count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 { false } else { ((op1 >> (count - 1)) & 0x0001) != 0 };
        let of = if count == 1 { (op1 & 0x8000) != 0 } else { false };
        self.update_flags_shr16(result, cf, of);
    }

    /// SHR r/m16, imm8
    pub fn shr_ew_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read16(instr);
        let result = if count >= 16 { 0 } else { op1 >> count };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 { false } else { ((op1 >> (count - 1)) & 0x0001) != 0 };
        let of = if count == 1 { (op1 & 0x8000) != 0 } else { false };
        self.update_flags_shr16(result, cf, of);
    }

    // =========================================================================
    // SHR - Shift Right Logical (32-bit)
    // =========================================================================
    
    /// SHR r/m32, 1
    pub fn shr_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read32(instr);
        let result = op1 >> 1;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 & 0x00000001) != 0;
        let of = (op1 & 0x80000000) != 0;
        self.update_flags_shr32(result, cf, of);
    }

    /// SHR r/m32, CL
    pub fn shr_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1 >> count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        let of = if count == 1 { (op1 & 0x80000000) != 0 } else { false };
        self.update_flags_shr32(result, cf, of);
    }

    /// SHR r/m32, imm8
    pub fn shr_ed_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1 >> count;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        let of = if count == 1 { (op1 & 0x80000000) != 0 } else { false };
        self.update_flags_shr32(result, cf, of);
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (preserves sign)
    // =========================================================================
    
    /// SAR r/m8, 1
    pub fn sar_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1_u, laddr) = self.shift_read8(instr);
        let op1 = op1_u as i8;
        let result = (op1 >> 1) as u8;
        self.shift_write8(instr, laddr, result);

        let cf = (op1 & 0x01) != 0;
        self.update_flags_sar8(result, cf);
    }

    /// SAR r/m8, CL
    pub fn sar_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1_u, laddr) = self.shift_read8(instr);
        let op1 = op1_u as i8;

        let result = if count >= 8 {
            if op1 < 0 { 0xFF } else { 0 }
        } else {
            ((op1 >> count) as u8)
        };
        self.shift_write8(instr, laddr, result);

        let cf = if count >= 8 { (op1 < 0) } else { ((op1 >> (count - 1)) & 0x01) != 0 };
        self.update_flags_sar8(result, cf);
    }

    /// SAR r/m16, 1
    pub fn sar_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1_u, laddr) = self.shift_read16(instr);
        let op1 = op1_u as i16;
        let result = (op1 >> 1) as u16;
        self.shift_write16(instr, laddr, result);

        let cf = (op1 & 0x0001) != 0;
        self.update_flags_sar16(result, cf);
    }

    /// SAR r/m16, CL
    pub fn sar_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1_u, laddr) = self.shift_read16(instr);
        let op1 = op1_u as i16;

        let result = if count >= 16 {
            if op1 < 0 { 0xFFFF } else { 0 }
        } else {
            (op1 >> count) as u16
        };
        self.shift_write16(instr, laddr, result);

        let cf = if count >= 16 { (op1 < 0) } else { ((op1 >> (count - 1)) & 0x0001) != 0 };
        self.update_flags_sar16(result, cf);
    }

    /// SAR r/m32, 1
    pub fn sar_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1_u, laddr) = self.shift_read32(instr);
        let op1 = op1_u as i32;
        let result = (op1 >> 1) as u32;
        self.shift_write32(instr, laddr, result);

        let cf = (op1 & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
    }

    /// SAR r/m32, CL
    pub fn sar_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1_u, laddr) = self.shift_read32(instr);
        let op1 = op1_u as i32;

        let result = (op1 >> count) as u32;
        self.shift_write32(instr, laddr, result);

        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
    }

    // =========================================================================
    // ROL - Rotate Left
    // =========================================================================
    
    /// ROL r/m8, 1
    pub fn rol_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read8(instr);
        let result = op1.rotate_left(1);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m8, CL
    pub fn rol_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x07; // Only low 3 bits for 8-bit rotate
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read8(instr);
        let result = op1.rotate_left(count as u32);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m16, 1
    pub fn rol_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read16(instr);
        let result = op1.rotate_left(1);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m16, CL
    pub fn rol_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x0F; // Only low 4 bits for 16-bit rotate
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read16(instr);
        let result = op1.rotate_left(count as u32);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    // =========================================================================
    // ROR - Rotate Right
    // =========================================================================
    
    /// ROR r/m8, 1
    pub fn ror_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read8(instr);
        let result = op1.rotate_right(1);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m8, CL
    pub fn ror_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x07;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read8(instr);
        let result = op1.rotate_right(count as u32);
        self.shift_write8(instr, laddr, result);

        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m16, 1
    pub fn ror_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read16(instr);
        let result = op1.rotate_right(1);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m16, CL
    pub fn ror_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x0F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read16(instr);
        let result = op1.rotate_right(count as u32);
        self.shift_write16(instr, laddr, result);

        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    // =========================================================================
    // 32-bit ROL/ROR
    // =========================================================================

    /// ROL r/m32, 1
    pub fn rol_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read32(instr);
        let result = op1.rotate_left(1);
        self.shift_write32(instr, laddr, result);

        let cf = result & 1 != 0;
        let of = ((result ^ (result >> 31)) & 1) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m32, CL
    pub fn rol_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1.rotate_left(count as u32);
        self.shift_write32(instr, laddr, result);

        let cf = result & 1 != 0;
        let of = if count == 1 { ((result ^ (result >> 31)) & 1) != 0 } else { false };
        self.set_cf_of(cf, of);
    }

    /// ROL r/m32, imm8
    pub fn rol_ed_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1.rotate_left(count);
        self.shift_write32(instr, laddr, result);

        let cf = result & 1 != 0;
        let of = if count == 1 { ((result ^ (result >> 31)) & 1) != 0 } else { false };
        self.set_cf_of(cf, of);
    }

    /// ROR r/m32, 1
    pub fn ror_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let (op1, laddr) = self.shift_read32(instr);
        let result = op1.rotate_right(1);
        self.shift_write32(instr, laddr, result);

        let cf = (result & 0x80000000) != 0;
        let of = ((result ^ (result << 1)) & 0x80000000) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m32, CL
    pub fn ror_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1.rotate_right(count as u32);
        self.shift_write32(instr, laddr, result);

        let cf = (result & 0x80000000) != 0;
        let of = if count == 1 { ((result ^ (result << 1)) & 0x80000000) != 0 } else { false };
        self.set_cf_of(cf, of);
    }

    /// ROR r/m32, imm8
    pub fn ror_ed_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1, laddr) = self.shift_read32(instr);
        let result = op1.rotate_right(count);
        self.shift_write32(instr, laddr, result);

        let cf = (result & 0x80000000) != 0;
        let of = if count == 1 { ((result ^ (result << 1)) & 0x80000000) != 0 } else { false };
        self.set_cf_of(cf, of);
    }

    // =========================================================================
    // Flag update helpers
    // =========================================================================

    fn update_flags_shl8(&mut self, result: u8, cf: bool, of: bool) {
        self.update_flags_logic8(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shl16(&mut self, result: u16, cf: bool, of: bool) {
        self.update_flags_logic16(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shl32(&mut self, result: u32, cf: bool, of: bool) {
        self.update_flags_logic32(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shr8(&mut self, result: u8, cf: bool, of: bool) {
        self.update_flags_logic8(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shr16(&mut self, result: u16, cf: bool, of: bool) {
        self.update_flags_logic16(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_shr32(&mut self, result: u32, cf: bool, of: bool) {
        self.update_flags_logic32(result);
        self.set_cf_of(cf, of);
    }

    fn update_flags_sar8(&mut self, result: u8, cf: bool) {
        self.update_flags_logic8(result);
        if cf { self.eflags |= 1; } else { self.eflags &= !1; }
        // OF is always 0 for SAR by 1
        self.eflags &= !(1 << 11);
    }

    fn update_flags_sar16(&mut self, result: u16, cf: bool) {
        self.update_flags_logic16(result);
        if cf { self.eflags |= 1; } else { self.eflags &= !1; }
        self.eflags &= !(1 << 11);
    }

    fn update_flags_sar32(&mut self, result: u32, cf: bool) {
        self.update_flags_logic32(result);
        if cf { self.eflags |= 1; } else { self.eflags &= !1; }
        self.eflags &= !(1 << 11);
    }

    // update_flags_logic8 and update_flags_logic16 are in cpu.rs

    fn set_cf_of(&mut self, cf: bool, of: bool) {
        if cf { self.eflags |= 1; } else { self.eflags &= !1; }
        if of { self.eflags |= 1 << 11; } else { self.eflags &= !(1 << 11); }
    }

    // =========================================================================
    // SHLD - Double Precision Shift Left
    // Based on Bochs shift32.cc:30-93
    // =========================================================================

    /// SHLD r/m32, r32, imm8
    /// Opcode: 0x0F 0xA4
    pub fn shld_ed_gd_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1_32, laddr) = self.shift_read32(instr);
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op1_32 << count) | (op2_32 >> (32 - count));
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_32 >> 31) != 0);
        self.set_cf_of(cf, of);
    }

    /// SHLD r/m32, r32, CL
    /// Opcode: 0x0F 0xA5
    pub fn shld_ed_gd_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1_32, laddr) = self.shift_read32(instr);
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op1_32 << count) | (op2_32 >> (32 - count));
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_32 >> 31) != 0);
        self.set_cf_of(cf, of);
    }

    // =========================================================================
    // SHRD - Double Precision Shift Right
    // Based on Bochs shift32.cc:97-161
    // =========================================================================

    /// SHRD r/m32, r32, imm8
    /// Opcode: 0x0F 0xAC
    pub fn shrd_ed_gd_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1_32, laddr) = self.shift_read32(instr);
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op2_32 << (32 - count)) | (op1_32 >> count);
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = (((result_32 << 1) ^ result_32) >> 31) != 0;
        self.set_cf_of(cf, of);
    }

    /// SHRD r/m32, r32, CL
    /// Opcode: 0x0F 0xAD
    pub fn shrd_ed_gd_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = (self.cl() & 0x1F) as u32;
        if count == 0 { return; }

        let (op1_32, laddr) = self.shift_read32(instr);
        let op2_32 = self.get_gpr32(instr.src() as usize);

        let result_32 = (op2_32 << (32 - count)) | (op1_32 >> count);
        self.shift_write32(instr, laddr, result_32);

        self.update_flags_logic32(result_32);
        let cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = (((result_32 << 1) ^ result_32) >> 31) != 0;
        self.set_cf_of(cf, of);
    }
}

