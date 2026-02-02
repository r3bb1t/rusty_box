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

    /// SHR r/m8, imm8
    pub fn shr_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = instr.ib() & 0x1F;
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

    /// SHR r/m32, imm8
    pub fn shr_ed_ib(&mut self, instr: &BxInstructionGenerated) {
        let count = (instr.ib() & 0x1F) as u32;
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

    // =========================================================================
    // SHLD - Double Precision Shift Left
    // Based on Bochs shift32.cc:30-93
    // =========================================================================

    /// SHLD r/m32, r32, imm8 (register form)
    /// Opcode: 0x0F 0xA4
    /// Original: bochs/cpu/shift32.cc:63-93 SHLD_EdGdR
    /// Shift destination left by count, filling from source register
    pub fn shld_ed_gd_ib(&mut self, instr: &BxInstructionGenerated) {
        let dst_idx = instr.meta_data[0] as usize;
        let src_idx = instr.meta_data[1] as usize;
        let count = (instr.ib() & 0x1F) as u32; // Use only 5 LSBs

        if count == 0 {
            // If count is 0, do nothing (but still clear upper 32 bits in 64-bit mode)
            // In 32-bit mode, this is a no-op
            return;
        }

        let op1_32 = self.get_gpr32(dst_idx);
        let op2_32 = self.get_gpr32(src_idx);

        // Shift op1 left by count, fill low bits from high bits of op2
        let result_32 = (op1_32 << count) | (op2_32 >> (32 - count));

        self.set_gpr32(dst_idx, result_32);

        // Set flags: OSZAPC (based on logic flags + CF/OF special)
        self.update_flags_logic32(result_32);

        // CF = bit shifted out of op1 (bit at position 32-count)
        let cf = ((op1_32 >> (32 - count)) & 0x1) != 0;

        // OF = CF XOR MSB of result
        let of = cf ^ ((result_32 >> 31) != 0);

        self.set_cf_of(cf, of);

        tracing::trace!("SHLD r{}, r{}, {}: {:#x} << {} | {:#x} >> {} = {:#x}",
            dst_idx, src_idx, count, op1_32, count, op2_32, 32 - count, result_32);
    }

    /// SHLD r/m32, r32, CL (register form)
    /// Opcode: 0x0F 0xA5
    /// Original: bochs/cpu/shift32.cc:63-93 SHLD_EdGdR
    /// Shift destination left by CL, filling from source register
    pub fn shld_ed_gd_cl(&mut self, instr: &BxInstructionGenerated) {
        let dst_idx = instr.meta_data[0] as usize;
        let src_idx = instr.meta_data[1] as usize;
        let count = (self.cl() & 0x1F) as u32; // Use only 5 LSBs

        if count == 0 {
            return;
        }

        let op1_32 = self.get_gpr32(dst_idx);
        let op2_32 = self.get_gpr32(src_idx);

        let result_32 = (op1_32 << count) | (op2_32 >> (32 - count));

        self.set_gpr32(dst_idx, result_32);

        self.update_flags_logic32(result_32);

        let cf = ((op1_32 >> (32 - count)) & 0x1) != 0;
        let of = cf ^ ((result_32 >> 31) != 0);

        self.set_cf_of(cf, of);

        tracing::trace!("SHLD r{}, r{}, CL({}): {:#x} -> {:#x}",
            dst_idx, src_idx, count, op1_32, result_32);
    }

    // =========================================================================
    // SHRD - Double Precision Shift Right
    // Based on Bochs shift32.cc:97-161
    // =========================================================================

    /// SHRD r/m32, r32, imm8 (register form)
    /// Opcode: 0x0F 0xAC
    /// Original: bochs/cpu/shift32.cc:130-161 SHRD_EdGdR
    /// Shift destination right by count, filling from source register
    pub fn shrd_ed_gd_ib(&mut self, instr: &BxInstructionGenerated) {
        let dst_idx = instr.meta_data[0] as usize;
        let src_idx = instr.meta_data[1] as usize;
        let count = (instr.ib() & 0x1F) as u32; // Use only 5 LSBs

        if count == 0 {
            return;
        }

        let op1_32 = self.get_gpr32(dst_idx);
        let op2_32 = self.get_gpr32(src_idx);

        // Shift op1 right by count, fill high bits from low bits of op2
        let result_32 = (op2_32 << (32 - count)) | (op1_32 >> count);

        self.set_gpr32(dst_idx, result_32);

        self.update_flags_logic32(result_32);

        // CF = bit shifted out of op1 (bit at position count-1)
        let cf = ((op1_32 >> (count - 1)) & 0x1) != 0;

        // OF = result_bit30 XOR result_bit31
        let of = (((result_32 << 1) ^ result_32) >> 31) != 0;

        self.set_cf_of(cf, of);

        tracing::trace!("SHRD r{}, r{}, {}: {:#x} >> {} | {:#x} << {} = {:#x}",
            dst_idx, src_idx, count, op1_32, count, op2_32, 32 - count, result_32);
    }

    /// SHRD r/m32, r32, CL (register form)
    /// Opcode: 0x0F 0xAD
    /// Original: bochs/cpu/shift32.cc:130-161 SHRD_EdGdR
    /// Shift destination right by CL, filling from source register
    pub fn shrd_ed_gd_cl(&mut self, instr: &BxInstructionGenerated) {
        let dst_idx = instr.meta_data[0] as usize;
        let src_idx = instr.meta_data[1] as usize;
        let count = (self.cl() & 0x1F) as u32;

        if count == 0 {
            return;
        }

        let op1_32 = self.get_gpr32(dst_idx);
        let op2_32 = self.get_gpr32(src_idx);

        let result_32 = (op2_32 << (32 - count)) | (op1_32 >> count);

        self.set_gpr32(dst_idx, result_32);

        self.update_flags_logic32(result_32);

        let cf = ((op1_32 >> (count - 1)) & 0x1) != 0;
        let of = (((result_32 << 1) ^ result_32) >> 31) != 0;

        self.set_cf_of(cf, of);

        tracing::trace!("SHRD r{}, r{}, CL({}): {:#x} -> {:#x}",
            dst_idx, src_idx, count, op1_32, result_32);
    }
}

