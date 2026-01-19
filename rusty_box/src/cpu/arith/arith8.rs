// 8-bit arithmetic operations: ADD, SUB, etc.
// Mirrors Bochs cpp/cpu/arith8.cc

use crate::cpu::decoder::{BxInstructionGenerated, BxSegregs};
use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::config::BxAddress;

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    /// Resolve effective address from ModRM (matches BX_CPU_RESOLVE_ADDR/BxResolve32)
    fn resolve_addr32(&self, instr: &BxInstructionGenerated) -> u32 {
        // Calculate: base + (index << scale) + displacement
        let base_reg = instr.sib_base() as usize;
        let mut eaddr = if base_reg < 16 {
            self.get_gpr32(base_reg)
        } else {
            0
        };
        
        eaddr = eaddr.wrapping_add(instr.displ32s() as u32);
        
        let index_reg = instr.sib_index();
        if index_reg != 4 {  // 4 means no index
            let index_val = if index_reg < 16 {
                self.get_gpr32(index_reg as usize)
            } else {
                0
            };
            let scale = instr.sib_scale();
            eaddr = eaddr.wrapping_add(index_val << scale);
        }
        
        // Apply address size mask
        if instr.as32_l() == 0 {
            // 16-bit address size
            eaddr & 0xFFFF
        } else {
            // 32-bit address size
            eaddr
        }
    }

    /// Get linear address from segment and offset
    fn get_laddr32_seg(&self, seg: BxSegregs, offset: u32) -> u32 {
        let seg_base = unsafe { self.sregs[seg as usize].cache.u.segment.base };
        (seg_base.wrapping_add(offset as u64)) as u32
    }

    /// Read byte from virtual address (matches read_virtual_byte)
    fn read_virtual_byte(&self, seg: BxSegregs, eaddr: u32) -> u8 {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        self.mem_read_byte(laddr as u64)
    }

    /// Read-Modify-Write: Read byte, return it and linear address for write back
    /// (matches read_RMW_virtual_byte)
    fn read_rmw_virtual_byte(&mut self, seg: BxSegregs, eaddr: u32) -> (u8, u32) {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        let val = self.mem_read_byte(laddr as u64);
        (val, laddr)
    }

    /// Write byte to linear address (for RMW operations, matches write_RMW_linear_byte)
    fn write_rmw_linear_byte(&mut self, laddr: u32, val: u8) {
        self.mem_write_byte(laddr as u64, val);
    }

    /// Read 8-bit register with extend8bitL support (matches BX_READ_8BIT_REGx)
    fn read_8bit_regx(&self, reg_idx: usize, extend8bit_l: u8) -> u8 {
        if extend8bit_l != 0 || (reg_idx & 4) == 0 {
            // Use low 8 bits (AL, BL, CL, DL, or extended regs)
            self.get_gpr8(reg_idx)
        } else {
            // Use high 8 bits (AH, BH, CH, DH)
            (self.get_gpr16((reg_idx - 4) + 8) >> 8) as u8
        }
    }

    /// Write 8-bit register with extend8bitL support (matches BX_WRITE_8BIT_REGx)
    fn write_8bit_regx(&mut self, reg_idx: usize, extend8bit_l: u8, val: u8) {
        if extend8bit_l != 0 || (reg_idx & 4) == 0 {
            // Write to low 8 bits
            self.set_gpr8(reg_idx, val);
        } else {
            // Write to high 8 bits
            let reg16_idx = (reg_idx - 4) + 8;
            let current = self.get_gpr16(reg16_idx);
            self.set_gpr16(reg16_idx, (current & !0xFF00) | ((val as u16) << 8));
        }
    }
}

/// ADD_EbGbM: ADD r/m8, r8 (memory form)
/// Opcode: 0x00, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::ADD_EbGbM
pub fn ADD_EbGbM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let (op1, laddr) = cpu.read_rmw_virtual_byte(seg, eaddr);
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let sum = op1.wrapping_add(op2);
    
    cpu.write_rmw_linear_byte(laddr, sum);
    cpu.update_flags_add8(op1, op2, sum);
    
    Ok(())
}

/// ADD_GbEbR: ADD r8, r/m8 (register form)
/// Opcode: 0x02, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::ADD_GbEbR
pub fn ADD_GbEbR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let sum = op1.wrapping_add(op2);
    
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);
    
    Ok(())
}

/// ADD_GbEbM: ADD r8, r/m8 (memory form)
/// Opcode: 0x02, ModRM: r8, r/m8 (memory)
/// Matches BX_CPU_C::ADD_GbEbM
pub fn ADD_GbEbM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_virtual_byte(seg, eaddr);
    let sum = op1.wrapping_add(op2);
    
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);
    
    Ok(())
}

/// ADD_EbGb: ADD r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn ADD_EbGb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: ADD r8, r8 - handled by ADD_GbEbR logic but operands swapped
        // For ADD r/m8, r8 in register mode: dst=r/m8, src=r8
        let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
        let sum = op1.wrapping_add(op2);
        
        cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
        cpu.update_flags_add8(op1, op2, sum);
        
        Ok(())
    } else {
        // Memory form
        ADD_EbGbM(cpu, instr)
    }
}

/// ADD_GbEb: ADD r8, r/m8
/// Dispatches to memory or register form based on ModRM
pub fn ADD_GbEb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form
        ADD_GbEbR(cpu, instr)
    } else {
        // Memory form
        ADD_GbEbM(cpu, instr)
    }
}

/// AND_EbGbM: AND r/m8, r8 (memory form)
/// Opcode: 0x20, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::AND_EbGbM
pub fn AND_EbGbM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let (op1, laddr) = cpu.read_rmw_virtual_byte(seg, eaddr);
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let result = op1 & op2;
    
    cpu.write_rmw_linear_byte(laddr, result);
    // Update flags for logical operation (AND)
    let sf = (result & 0x80) != 0;
    let zf = result == 0;
    let pf = result.count_ones() % 2 == 0;
    const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
    cpu.eflags &= !MASK;
    if pf { cpu.eflags |= 1 << 2; }
    if zf { cpu.eflags |= 1 << 6; }
    if sf { cpu.eflags |= 1 << 7; }
    
    Ok(())
}

/// AND_GbEbR: AND r8, r8 (register form)
/// Opcode: 0x20, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::AND_GbEbR
pub fn AND_GbEbR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let result = op1 & op2;
    
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), result);
    // Update flags for logical operation (AND)
    let sf = (result & 0x80) != 0;
    let zf = result == 0;
    let pf = result.count_ones() % 2 == 0;
    const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
    cpu.eflags &= !MASK;
    if pf { cpu.eflags |= 1 << 2; }
    if zf { cpu.eflags |= 1 << 6; }
    if sf { cpu.eflags |= 1 << 7; }
    
    Ok(())
}

/// AND_EbGb: AND r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn AND_EbGb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: AND r8, r8 - handled by AND_GbEbR logic but operands swapped
        // For AND r/m8, r8 in register mode: dst=r/m8, src=r8
        let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
        let result = op1 & op2;
        
        cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), result);
        // Update flags for logical operation (AND)
        let sf = (result & 0x80) != 0;
        let zf = result == 0;
        let pf = result.count_ones() % 2 == 0;
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
        cpu.eflags &= !MASK;
        if pf { cpu.eflags |= 1 << 2; }
        if zf { cpu.eflags |= 1 << 6; }
        if sf { cpu.eflags |= 1 << 7; }
        
        Ok(())
    } else {
        // Memory form
        AND_EbGbM(cpu, instr)
    }
}

/// ADC_EbGbM: ADC r/m8, r8 (memory form)
/// Opcode: 0x10, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::ADC_EbGbM
pub fn ADC_EbGbM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let (op1, laddr) = cpu.read_rmw_virtual_byte(seg, eaddr);
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);
    
    cpu.write_rmw_linear_byte(laddr, sum);
    cpu.update_flags_add8(op1, op2, sum);
    
    Ok(())
}

/// ADC_GbEbR: ADC r8, r8 (register form)
/// Opcode: 0x10, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::ADC_GbEbR
pub fn ADC_GbEbR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);
    
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);
    
    Ok(())
}

/// ADC_EbGb: ADC r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn ADC_EbGb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: ADC r8, r8 - handled by ADC_GbEbR logic but operands swapped
        // For ADC r/m8, r8 in register mode: dst=r/m8, src=r8
        let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
        let cf = cpu.get_cf() as u8;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);
        
        cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
        cpu.update_flags_add8(op1, op2, sum);
        
        Ok(())
    } else {
        // Memory form
        ADC_EbGbM(cpu, instr)
    }
}

/// ADD_EbIbR: ADD r/m8, imm8 (register form)
/// Opcode: 0x80/0 or 0x83/0 (8-bit)
/// Matches BX_CPU_C::ADD_EbIbR
pub fn ADD_EbIbR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = instr.ib();
    let sum = op1.wrapping_add(op2);
    
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);
    
    Ok(())
}

/// ADD_EbIb: ADD r/m8, imm8
/// Dispatches to register form (memory form would use ADD_EbIbM if needed)
/// This covers ADD AL, imm8 (AddAlib opcode)
pub fn ADD_EbIb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // For register form (including AL), use ADD_EbIbR
    ADD_EbIbR(cpu, instr)
}
