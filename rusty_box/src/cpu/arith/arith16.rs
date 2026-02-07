// 16-bit arithmetic operations: ADD, ADC, SUB, etc.
// Mirrors Bochs cpp/cpu/arith16.cc

use crate::cpu::decoder::{BxInstructionGenerated, BxSegregs};
use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::config::BxAddress;

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    /// Get linear address from segment and offset (helper for 16-bit operations)
    fn get_laddr32_seg_arith16(&self, seg: BxSegregs, offset: u32) -> u32 {
        let seg_base = unsafe { self.sregs[seg as usize].cache.u.segment.base };
        (seg_base.wrapping_add(offset as u64)) as u32
    }

    /// Read 16-bit word from virtual address (matches read_virtual_word)
    fn read_virtual_word_arith16(&self, seg: BxSegregs, eaddr: u32) -> u16 {
        let laddr = self.get_laddr32_seg_arith16(seg, eaddr);
        self.mem_read_word(laddr as u64)
    }
}

/// ADC_GwEwR: ADC r16, r16 (register form)
/// Opcode: 0x13, ModRM: r16, r/m16 (register)
/// Matches BX_CPU_C::ADC_GwEwR
pub fn ADC_GwEwR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);
    
    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);
    
    Ok(())
}

/// ADC_GwEwM: ADC r16, r/m16 (memory form)
/// Opcode: 0x13, ModRM: r16, r/m16 (memory)
/// Matches BX_CPU_C::ADC_GwEwM
pub fn ADC_GwEwM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve address manually (same logic as resolve_addr32 in arith8)
    let base_reg = instr.sib_base() as usize;
    let mut eaddr = if base_reg < 16 {
        cpu.get_gpr32(base_reg)
    } else {
        0
    };
    
    eaddr = eaddr.wrapping_add(instr.displ32s() as u32);
    
    let index_reg = instr.sib_index();
    if index_reg != 4 {  // 4 means no index
        let index_val = if index_reg < 16 {
            cpu.get_gpr32(index_reg as usize)
        } else {
            0
        };
        let scale = instr.sib_scale();
        eaddr = eaddr.wrapping_add(index_val << scale);
    }
    
    // Apply address size mask
    let eaddr = if instr.as32_l() == 0 {
        // 16-bit address size
        eaddr & 0xFFFF
    } else {
        // 32-bit address size
        eaddr
    };
    
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.read_virtual_word_arith16(seg, eaddr);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);
    
    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);
    
    Ok(())
}

/// ADC_GwEw: ADC r16, r/m16
/// Dispatches to memory or register form based on ModRM
pub fn ADC_GwEw<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form
        ADC_GwEwR(cpu, instr)
    } else {
        // Memory form
        ADC_GwEwM(cpu, instr)
    }
}

/// ADD_EwIbR: ADD r/m16, imm8 (sign-extended, register form)
/// Opcode: 0x83/0
/// Matches pattern for ADD r16, imm8 (sign-extended)
pub fn ADD_EwIbR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.meta_data[0] as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = (instr.ib() as i8 as i16 as u16); // Sign-extend imm8 to u16
    let result = op1.wrapping_add(op2);

    cpu.set_gpr16(dst, result);
    cpu.update_flags_add16(op1, op2, result);

    Ok(())
}

/// ADD_EwIwR: ADD r16, imm16 (register form)
/// Opcode: 0x81/0
/// Based on BX_CPU_C::ADD_EwIwR in arith16.cc
pub fn ADD_EwIwR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1_16 = cpu.get_gpr16(dst);
    let op2_16 = instr.iw();  // Read 16-bit immediate
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(dst, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_EwIwM: ADD m16, imm16 (memory form)
/// Opcode: 0x81/0
/// Based on BX_CPU_C::ADD_EwIwM in arith16.cc
pub fn ADD_EwIwM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve address manually (same logic as ADC_GwEwM)
    let base_reg = instr.sib_base() as usize;
    let mut eaddr = if base_reg < 16 {
        cpu.get_gpr32(base_reg)
    } else {
        0
    };

    eaddr = eaddr.wrapping_add(instr.displ32s() as u32);

    let index_reg = instr.sib_index();
    if index_reg != 4 {  // 4 means no index
        let index_val = if index_reg < 16 {
            cpu.get_gpr32(index_reg as usize)
        } else {
            0
        };
        let scale = instr.sib_scale();
        eaddr = eaddr.wrapping_add(index_val << scale);
    }

    // Apply address size mask
    let eaddr = if instr.as32_l() == 0 {
        eaddr & 0xFFFF  // 16-bit address size
    } else {
        eaddr           // 32-bit address size
    };

    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let op1_16 = cpu.read_virtual_word_arith16(seg, eaddr);
    let op2_16 = instr.iw();  // Read 16-bit immediate
    let sum_16 = op1_16.wrapping_add(op2_16);

    // Write result back to memory
    let laddr = cpu.get_laddr32_seg_arith16(seg, eaddr);
    cpu.mem_write_word(laddr as u64, sum_16);

    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_EwIw: ADD r/m16, imm16
/// Dispatches to memory or register form based on ModRM
pub fn ADD_EwIw<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form
        ADD_EwIwR(cpu, instr)
    } else {
        // Memory form
        ADD_EwIwM(cpu, instr)
    }
}

/// ADD_EwGwM: ADD r/m16, r16 (memory form)
/// Original: bochs/cpu/arith16.cc lines 43-56
/// Opcode: 0x01, ModRM: r/m16, r16 (memory)
/// Matches BX_CPU_C::ADD_EwGwM
pub fn ADD_EwGwM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve address manually (same logic as other memory operations)
    let base_reg = instr.sib_base() as usize;
    let mut eaddr = if base_reg < 16 {
        cpu.get_gpr32(base_reg)
    } else {
        0
    };

    eaddr = eaddr.wrapping_add(instr.displ32s() as u32);

    let index_reg = instr.sib_index();
    if index_reg != 4 {  // 4 means no index
        let index_val = if index_reg < 16 {
            cpu.get_gpr32(index_reg as usize)
        } else {
            0
        };
        let scale = instr.sib_scale();
        eaddr = eaddr.wrapping_add(index_val << scale);
    }

    // Apply address size mask
    let eaddr = if instr.as32_l() == 0 {
        eaddr & 0xFFFF  // 16-bit address size
    } else {
        eaddr           // 32-bit address size
    };

    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let op1_16 = cpu.read_virtual_word_arith16(seg, eaddr);  // Read from memory
    let op2_16 = cpu.get_gpr16(instr.src() as usize);        // Read from register
    let sum_16 = op1_16.wrapping_add(op2_16);

    // Write result back to memory
    let laddr = cpu.get_laddr32_seg_arith16(seg, eaddr);
    cpu.mem_write_word(laddr as u64, sum_16);

    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_EwGwR: ADD r16, r16 (register form, both operands are registers)
/// Original: bochs/cpu/arith16.cc lines 58-69 (ADD_GwEwR)
/// Opcode: 0x01, ModRM: r16, r16 (register)
/// Matches BX_CPU_C::ADD_GwEwR (reused for register form)
pub fn ADD_EwGwR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_EwGw: ADD r/m16, r16
/// Dispatches to memory or register form based on ModRM
pub fn ADD_EwGw<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: both operands are registers
        ADD_EwGwR(cpu, instr)
    } else {
        // Memory form: destination is memory, source is register
        ADD_EwGwM(cpu, instr)
    }
}

/// ADC_EwGwM: ADC r/m16, r16 (memory form)
/// Original: bochs/cpu/arith16.cc lines 86-99
/// Opcode: 0x11, ModRM: r/m16, r16 (memory)
pub fn ADC_EwGwM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve address manually (same logic as other memory operations)
    let base_reg = instr.sib_base() as usize;
    let mut eaddr = if base_reg < 16 {
        cpu.get_gpr32(base_reg)
    } else {
        0
    };

    eaddr = eaddr.wrapping_add(instr.displ32s() as u32);

    let index_reg = instr.sib_index();
    if index_reg != 4 {  // 4 means no index
        let index_val = if index_reg < 16 {
            cpu.get_gpr32(index_reg as usize)
        } else {
            0
        };
        let scale = instr.sib_scale();
        eaddr = eaddr.wrapping_add(index_val << scale);
    }

    // Apply address size mask
    let eaddr = if instr.as32_l() == 0 {
        // 16-bit address size
        eaddr & 0xFFFF
    } else {
        // 32-bit address size
        eaddr
    };

    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };
    let op1_16 = cpu.read_virtual_word_arith16(seg, eaddr);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);

    // Write result back to memory
    let laddr = cpu.get_laddr32_seg_arith16(seg, eaddr);
    cpu.mem_write_word(laddr as u64, sum_16);

    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADC_EwGwR: ADC r16, r16 (register form)
/// Based on the pattern of ADC where destination is Ew (r/m16) and source is Gw (r16)
/// When both operands are registers, this is functionally the same as ADC r16, r16
pub fn ADC_EwGwR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADC_EwGw: ADC r/m16, r16
/// Dispatches to memory or register form based on ModRM
pub fn ADC_EwGw<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form
        ADC_EwGwR(cpu, instr)
    } else {
        // Memory form
        ADC_EwGwM(cpu, instr)
    }
}

// =========================================================================
// CMP - Compare (16-bit)
// =========================================================================

/// CMP_EwGwR: CMP r/m16, r16 (register form)
/// Performs subtraction without storing result, only sets flags
/// Opcode: 0x39, ModRM: r16, r16
pub fn CMP_EwGwR<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let result = op1_16.wrapping_sub(op2_16);

    cpu.update_flags_sub16(op1_16, op2_16, result);

    Ok(())
}

/// CMP_EwGwM: CMP r/m16, r16 (memory form)
/// Opcode: 0x39, ModRM: r/m16 (memory), r16
pub fn CMP_EwGwM<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve address
    let eaddr = cpu.resolve_addr32(instr);
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };

    let op1_16 = cpu.read_virtual_word_arith16(seg, eaddr);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let result = op1_16.wrapping_sub(op2_16);

    cpu.update_flags_sub16(op1_16, op2_16, result);

    Ok(())
}

/// CMP_EwGw: CMP r/m16, r16 - Dispatcher
pub fn CMP_EwGw<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_EwGwR(cpu, instr)
    } else {
        CMP_EwGwM(cpu, instr)
    }
}

// =========================================================================
// ADD - Accumulator optimized forms
// =========================================================================

/// ADD_Axiw: ADD AX, imm16
/// Optimized form for accumulator
/// Opcode: 0x05
pub fn ADD_Axiw<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let ax = cpu.ax();
    let imm16 = instr.iw();
    let result = ax.wrapping_add(imm16);

    cpu.set_ax(result);
    cpu.update_flags_add16(ax, imm16, result);

    Ok(())
}
