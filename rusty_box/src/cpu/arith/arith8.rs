// 8-bit arithmetic operations: ADD, SUB, etc.
// Mirrors Bochs cpp/cpu/arith8.cc

use crate::cpu::decoder::{BxInstructionGenerated, BxSegregs};
use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::config::BxAddress;

// Helper methods are defined in logical8.rs and data_xfer_ext.rs
// Free functions below use cpu.resolve_addr32(), cpu.read_8bit_regx(), etc.
// which call the public methods from those modules

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

/// INC_Eb: INC r/m8 (register form)
/// Opcode: 0xFE/0
/// Original: bochs/cpu/arith8.cc INC_EbR
/// Increment 8-bit register by 1
pub fn INC_Eb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.read_8bit_regx(dst, instr.extend8bit_l());
    let result = op1.wrapping_add(1);
    cpu.write_8bit_regx(dst, instr.extend8bit_l(), result);

    // INC affects OF, SF, ZF, AF, PF but NOT CF
    let zf = result == 0;
    let sf = (result & 0x80) != 0;
    let of = result == 0x80; // Overflow if we wrapped from 0x7F to 0x80
    let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
    let pf = result.count_ones() % 2 == 0;

    // Update all flags except CF (bit 0)
    const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
    cpu.eflags &= !MASK;
    if pf { cpu.eflags |= 1 << 2; }
    if af { cpu.eflags |= 1 << 4; }
    if zf { cpu.eflags |= 1 << 6; }
    if sf { cpu.eflags |= 1 << 7; }
    if of { cpu.eflags |= 1 << 11; }

    Ok(())
}

/// DEC_Eb: DEC r/m8 (register form)
/// Opcode: 0xFE/1
/// Original: bochs/cpu/arith8.cc DEC_EbR
/// Decrement 8-bit register by 1
pub fn DEC_Eb<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.read_8bit_regx(dst, instr.extend8bit_l());
    let result = op1.wrapping_sub(1);
    cpu.write_8bit_regx(dst, instr.extend8bit_l(), result);

    // DEC affects OF, SF, ZF, AF, PF but NOT CF
    let zf = result == 0;
    let sf = (result & 0x80) != 0;
    let of = result == 0x7F; // Overflow if we wrapped from 0x80 to 0x7F
    let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
    let pf = result.count_ones() % 2 == 0;

    const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
    cpu.eflags &= !MASK;
    if pf { cpu.eflags |= 1 << 2; }
    if af { cpu.eflags |= 1 << 4; }
    if zf { cpu.eflags |= 1 << 6; }
    if sf { cpu.eflags |= 1 << 7; }
    if of { cpu.eflags |= 1 << 11; }

    Ok(())
}
