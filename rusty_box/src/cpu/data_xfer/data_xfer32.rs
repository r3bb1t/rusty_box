// 32-bit data transfer operations: MOV, etc.
// Mirrors Bochs cpp/cpu/data_xfer32.cc

use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::cpu::decoder::BxInstructionGenerated;

/// MOV_GdEd_R: MOV r32, r/m32 (register form)
/// Opcode: 0x8B, ModRM: r32, r/m32 (register)
/// meta_data[0] = destination register
/// meta_data[1] = source register
pub fn MOV_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;
    
    let val = cpu.get_gpr32(src_idx);
    cpu.set_gpr32(dst_idx, val);
}

/// MOV_EdGd_R: MOV r/m32, r32 (register form)
/// Opcode: 0x89, ModRM: r/m32, r32 (register)
/// meta_data[0] = destination register
/// meta_data[1] = source register
pub fn MOV_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;
    
    let val = cpu.get_gpr32(src_idx);
    cpu.set_gpr32(dst_idx, val);
}

/// MOV_EdId_R: MOV r/m32, imm32 (register form)
/// Opcode: 0xC7, ModRM: r/m32, imm32 (register)
/// meta_data[0] = destination register
/// Immediate value stored in operand_data.Id
pub fn MOV_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let imm: u32 = instr.modrm_form.operand_data.id();
    
    cpu.set_gpr32(dst_idx, imm);
}

/// MOV_EAX_Id: MOV EAX, imm32 (register direct)
/// Opcodes: 0xB8-0xBF (0xB8 + register index)
/// meta_data[0] = register index
pub fn MOV_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let imm: u32 = instr.modrm_form.operand_data.id();
    
    cpu.set_gpr32(dst_idx, imm);
}
