//! Shift and rotate instructions for x86 CPU emulation
//!
//! Based on Bochs shift8.cc, shift16.cc, shift32.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements SHL, SHR, SAR, ROL, ROR, RCL, RCR instructions

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // SHL - Shift Left (8-bit)
    // =========================================================================
    
    /// SHL r/m8, 1
    pub fn shl_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        let result = op1 << 1;
        self.set_gpr8(dst, result);
        
        let cf = (op1 & 0x80) != 0;
        let of = ((result ^ op1) & 0x80) != 0; // OF = CF XOR MSB of result
        self.update_flags_shl8(result, cf, of);
    }

    /// SHL r/m8, CL
    pub fn shl_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        
        let result = if count >= 8 { 0 } else { op1 << count };
        self.set_gpr8(dst, result);
        
        let cf = if count >= 8 { false } else { ((op1 << (count - 1)) & 0x80) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x80) != 0 } else { false };
        self.update_flags_shl8(result, cf, of);
    }

    /// SHL r/m8, imm8
    pub fn shl_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = instr.ib() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        
        let result = if count >= 8 { 0 } else { op1 << count };
        self.set_gpr8(dst, result);
        
        let cf = if count >= 8 { false } else { ((op1 << (count - 1)) & 0x80) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x80) != 0 } else { false };
        self.update_flags_shl8(result, cf, of);
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (8-bit)
    // =========================================================================
    
    /// SAR r/m8, imm8
    /// Opcode: 0xC0/7 or 0xD0/7 with imm8
    /// Matches BX_CPU_C::SAR_EbR (for imm8 case)
    pub fn sar_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = instr.ib() & 0x1F;
        
        if count == 0 {
            return;
        }
        
        let dst = instr.meta_data[0] as usize;
        let op1_8 = self.get_gpr8(dst);
        let result_8 = ((op1_8 as i8) >> count) as u8;
        
        self.set_gpr8(dst, result_8);
        
        let cf = (((op1_8 as i8) >> (count - 1)) & 0x1) != 0;
        
        // SET_FLAGS_OSZAPC_LOGIC_8(result_8) + set CF
        self.update_flags_logic8(result_8);
        self.set_cf_of(cf, false);  // CF from shift, OF = 0 for SAR
    }

    // =========================================================================
    // SHL - Shift Left (16-bit)
    // =========================================================================
    
    /// SHL r/m16, 1
    pub fn shl_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1 << 1;
        self.set_gpr16(dst, result);
        
        let cf = (op1 & 0x8000) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.update_flags_shl16(result, cf, of);
    }

    /// SHL r/m16, CL
    pub fn shl_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        
        let result = if count >= 16 { 0 } else { op1 << count };
        self.set_gpr16(dst, result);
        
        let cf = if count >= 16 { false } else { ((op1 << (count - 1)) & 0x8000) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x8000) != 0 } else { false };
        self.update_flags_shl16(result, cf, of);
    }

    /// SHL r/m16, imm8
    pub fn shl_ew_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        
        let result = if count >= 16 { 0 } else { op1 << count };
        self.set_gpr16(dst, result);
        
        let cf = if count >= 16 { false } else { ((op1 << (count - 1)) & 0x8000) != 0 };
        let of = if count == 1 { ((result ^ op1) & 0x8000) != 0 } else { false };
        self.update_flags_shl16(result, cf, of);
    }

    // =========================================================================
    // SHL - Shift Left (32-bit)
    // =========================================================================
    
    /// SHL r/m32, 1
    pub fn shl_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst);
        let result = op1 << 1;
        self.set_gpr32(dst, result);
        
        let cf = (op1 & 0x80000000) != 0;
        let of = ((result ^ op1) & 0x80000000) != 0;
        self.update_flags_shl32(result, cf, of);
    }

    /// SHL r/m32, CL
    pub fn shl_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst);
        
        let result = op1 << count;
        self.set_gpr32(dst, result);
        
        let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
        let of = if count == 1 { ((result ^ op1) & 0x80000000) != 0 } else { false };
        self.update_flags_shl32(result, cf, of);
    }

    /// SHL r/m32, imm8
    pub fn shl_ed_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst);
        
        let result = op1 << count;
        self.set_gpr32(dst, result);
        
        let cf = ((op1 << (count - 1)) & 0x80000000) != 0;
        let of = if count == 1 { ((result ^ op1) & 0x80000000) != 0 } else { false };
        self.update_flags_shl32(result, cf, of);
    }

    // =========================================================================
    // SHR - Shift Right Logical (8-bit)
    // =========================================================================
    
    /// SHR r/m8, 1
    pub fn shr_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        let result = op1 >> 1;
        self.set_gpr8(dst, result);
        
        let cf = (op1 & 0x01) != 0;
        let of = (op1 & 0x80) != 0; // OF = MSB of original operand
        self.update_flags_shr8(result, cf, of);
    }

    /// SHR r/m8, CL
    pub fn shr_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        
        let result = if count >= 8 { 0 } else { op1 >> count };
        self.set_gpr8(dst, result);
        
        let cf = if count >= 8 { false } else { ((op1 >> (count - 1)) & 0x01) != 0 };
        let of = if count == 1 { (op1 & 0x80) != 0 } else { false };
        self.update_flags_shr8(result, cf, of);
    }

    // =========================================================================
    // SHR - Shift Right Logical (16-bit)
    // =========================================================================
    
    /// SHR r/m16, 1
    pub fn shr_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1 >> 1;
        self.set_gpr16(dst, result);
        
        let cf = (op1 & 0x0001) != 0;
        let of = (op1 & 0x8000) != 0;
        self.update_flags_shr16(result, cf, of);
    }

    /// SHR r/m16, CL
    pub fn shr_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        
        let result = if count >= 16 { 0 } else { op1 >> count };
        self.set_gpr16(dst, result);
        
        let cf = if count >= 16 { false } else { ((op1 >> (count - 1)) & 0x0001) != 0 };
        let of = if count == 1 { (op1 & 0x8000) != 0 } else { false };
        self.update_flags_shr16(result, cf, of);
    }

    /// SHR r/m16, imm8
    pub fn shr_ew_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        
        let result = if count >= 16 { 0 } else { op1 >> count };
        self.set_gpr16(dst, result);
        
        let cf = if count >= 16 { false } else { ((op1 >> (count - 1)) & 0x0001) != 0 };
        let of = if count == 1 { (op1 & 0x8000) != 0 } else { false };
        self.update_flags_shr16(result, cf, of);
    }

    // =========================================================================
    // SHR - Shift Right Logical (32-bit)
    // =========================================================================
    
    /// SHR r/m32, 1
    pub fn shr_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst);
        let result = op1 >> 1;
        self.set_gpr32(dst, result);
        
        let cf = (op1 & 0x00000001) != 0;
        let of = (op1 & 0x80000000) != 0;
        self.update_flags_shr32(result, cf, of);
    }

    /// SHR r/m32, CL
    pub fn shr_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst);
        
        let result = op1 >> count;
        self.set_gpr32(dst, result);
        
        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        let of = if count == 1 { (op1 & 0x80000000) != 0 } else { false };
        self.update_flags_shr32(result, cf, of);
    }

    // =========================================================================
    // SAR - Shift Arithmetic Right (preserves sign)
    // =========================================================================
    
    /// SAR r/m8, 1
    pub fn sar_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst) as i8;
        let result = (op1 >> 1) as u8;
        self.set_gpr8(dst, result);
        
        let cf = (op1 & 0x01) != 0;
        self.update_flags_sar8(result, cf);
    }

    /// SAR r/m8, CL
    pub fn sar_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst) as i8;
        
        let result = if count >= 8 { 
            if op1 < 0 { 0xFF } else { 0 }
        } else { 
            ((op1 >> count) as u8)
        };
        self.set_gpr8(dst, result);
        
        let cf = if count >= 8 { (op1 < 0) } else { ((op1 >> (count - 1)) & 0x01) != 0 };
        self.update_flags_sar8(result, cf);
    }

    /// SAR r/m16, 1
    pub fn sar_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst) as i16;
        let result = (op1 >> 1) as u16;
        self.set_gpr16(dst, result);
        
        let cf = (op1 & 0x0001) != 0;
        self.update_flags_sar16(result, cf);
    }

    /// SAR r/m16, CL
    pub fn sar_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst) as i16;
        
        let result = if count >= 16 {
            if op1 < 0 { 0xFFFF } else { 0 }
        } else {
            (op1 >> count) as u16
        };
        self.set_gpr16(dst, result);
        
        let cf = if count >= 16 { (op1 < 0) } else { ((op1 >> (count - 1)) & 0x0001) != 0 };
        self.update_flags_sar16(result, cf);
    }

    /// SAR r/m32, 1
    pub fn sar_ed_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst) as i32;
        let result = (op1 >> 1) as u32;
        self.set_gpr32(dst, result);
        
        let cf = (op1 & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
    }

    /// SAR r/m32, CL
    pub fn sar_ed_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x1F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr32(dst) as i32;
        
        let result = (op1 >> count) as u32;
        self.set_gpr32(dst, result);
        
        let cf = ((op1 >> (count - 1)) & 0x00000001) != 0;
        self.update_flags_sar32(result, cf);
    }

    // =========================================================================
    // ROL - Rotate Left
    // =========================================================================
    
    /// ROL r/m8, 1
    pub fn rol_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        let result = op1.rotate_left(1);
        self.set_gpr8(dst, result);
        
        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m8, CL
    pub fn rol_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x07; // Only low 3 bits for 8-bit rotate
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        let result = op1.rotate_left(count as u32);
        self.set_gpr8(dst, result);
        
        let cf = (result & 0x01) != 0;
        let of = ((result ^ op1) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m16, 1
    pub fn rol_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.rotate_left(1);
        self.set_gpr16(dst, result);
        
        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROL r/m16, CL
    pub fn rol_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x0F; // Only low 4 bits for 16-bit rotate
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.rotate_left(count as u32);
        self.set_gpr16(dst, result);
        
        let cf = (result & 0x0001) != 0;
        let of = ((result ^ op1) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    // =========================================================================
    // ROR - Rotate Right
    // =========================================================================
    
    /// ROR r/m8, 1
    pub fn ror_eb_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        let result = op1.rotate_right(1);
        self.set_gpr8(dst, result);
        
        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m8, CL
    pub fn ror_eb_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x07;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr8(dst);
        let result = op1.rotate_right(count as u32);
        self.set_gpr8(dst, result);
        
        let cf = (result & 0x80) != 0;
        let of = ((result ^ (result << 1)) & 0x80) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m16, 1
    pub fn ror_ew_1(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.rotate_right(1);
        self.set_gpr16(dst, result);
        
        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
        self.set_cf_of(cf, of);
    }

    /// ROR r/m16, CL
    pub fn ror_ew_cl(&mut self, instr: &BxInstructionGenerated) {
        let count = self.cl() & 0x0F;
        if count == 0 { return; }
        
        let dst = instr.meta_data[0] as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.rotate_right(count as u32);
        self.set_gpr16(dst, result);
        
        let cf = (result & 0x8000) != 0;
        let of = ((result ^ (result << 1)) & 0x8000) != 0;
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
}

