// 32-bit arithmetic operations: ADD, SUB, etc.
// Mirrors Bochs cpp/cpu/arith32.cc

use crate::cpu::decoder::BxInstructionGenerated;
use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// ADD_GdEd_R: ADD r32, r/m32 (register form)
/// Opcode: 0x03, ModRM: r32, r/m32 (register)
/// meta_data[0] = destination register
/// meta_data[1] = source register
pub fn ADD_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_add(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_add32(dst_val, src_val, result);
}

/// ADD_EdGd_R: ADD r/m32, r32 (register form)
/// Opcode: 0x01, ModRM: r/m32, r32 (register)
/// meta_data[0] = destination register  
/// meta_data[1] = source register
pub fn ADD_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_add(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_add32(dst_val, src_val, result);
}

/// ADD_EAX_Id: ADD EAX, imm32
/// Opcode: 0x05
/// Immediate value stored in operand_data.Id
pub fn ADD_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let eax = cpu.eax();
    let imm: u32 = instr.modrm_form.operand_data.id();
    let result = eax.wrapping_add(imm);

    cpu.set_eax(result);
    cpu.update_flags_add32(eax, imm, result);
}

/// SUB_GdEd_R: SUB r32, r/m32 (register form)
/// Opcode: 0x2B, ModRM: r32, r/m32 (register)
pub fn SUB_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_sub(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_sub32(dst_val, src_val, result);
}

/// SUB_EdGd_R: SUB r/m32, r32 (register form)
/// Opcode: 0x29, ModRM: r/m32, r32 (register)
pub fn SUB_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_sub(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_sub32(dst_val, src_val, result);
}

/// SUB_EAX_Id: SUB EAX, imm32
/// Opcode: 0x2D
pub fn SUB_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let eax = cpu.eax();
    let imm: u32 = instr.modrm_form.operand_data.id();
    let result = eax.wrapping_sub(imm);

    cpu.set_eax(result);
    cpu.update_flags_sub32(eax, imm, result);
}

/// ADC_EdGd_R: ADC r/m32, r32 (register form)
/// Original: bochs/cpu/arith32.cc ADC_EdGd (register case)
/// Opcode: 0x11, ModRM: r/m32, r32 (register)
pub fn ADC_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let op1_32 = cpu.get_gpr32(dst_idx);
    let op2_32 = cpu.get_gpr32(src_idx);
    let cf = cpu.get_cf() as u32;
    let sum_32 = op1_32.wrapping_add(op2_32).wrapping_add(cf);

    cpu.set_gpr32(dst_idx, sum_32);
    cpu.update_flags_add32(op1_32, op2_32, sum_32);
}

/// ADC_GdEd_R: ADC r32, r/m32 (register form)
/// Original: bochs/cpu/arith32.cc ADC_GdEd (register case)
/// Opcode: 0x13, ModRM: r32, r/m32 (register)
pub fn ADC_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let op1_32 = cpu.get_gpr32(dst_idx);
    let op2_32 = cpu.get_gpr32(src_idx);
    let cf = cpu.get_cf() as u32;
    let sum_32 = op1_32.wrapping_add(op2_32).wrapping_add(cf);

    cpu.set_gpr32(dst_idx, sum_32);
    cpu.update_flags_add32(op1_32, op2_32, sum_32);
}
