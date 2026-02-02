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

/// MOVZX_GdEb: MOVZX r32, r/m8
/// Opcode: 0x0F 0xB6, ModRM: r32, r/m8
/// Original: bochs/cpu/data_xfer32.cc:110-130 MOVZX_GdEbM/MOVZX_GdEbR
/// Zero extend byte operand into dword destination
pub fn MOVZX_GdEb<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_reg = instr.meta_data[0] as usize;
    let src_reg = instr.meta_data[1] as usize;

    // Read 8-bit operand (handles both memory and register based on decoder metadata)
    let op2_8 = cpu.get_gpr8(src_reg);

    // Zero extend byte op2 into dword op1
    cpu.set_gpr32(dst_reg, op2_8 as u32);

    tracing::trace!("MOVZX32 r{}, r{}b: {:#04x} -> {:#010x}", dst_reg, src_reg, op2_8, op2_8 as u32);
}

/// MOVZX_GdEw: MOVZX r32, r/m16
/// Opcode: 0x0F 0xB7, ModRM: r32, r/m16
/// Original: bochs/cpu/data_xfer32.cc:132-152 MOVZX_GdEwM/MOVZX_GdEwR
/// Zero extend word operand into dword destination
pub fn MOVZX_GdEw<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_reg = instr.meta_data[0] as usize;
    let src_reg = instr.meta_data[1] as usize;

    // Read 16-bit operand (handles both memory and register based on decoder metadata)
    let op2_16 = cpu.get_gpr16(src_reg);

    // Zero extend word op2 into dword op1
    cpu.set_gpr32(dst_reg, op2_16 as u32);

    tracing::trace!("MOVZX32 r{}, r{}w: {:#06x} -> {:#010x}", dst_reg, src_reg, op2_16, op2_16 as u32);
}

/// MOV_EAXOd: MOV EAX, moffs32
/// Opcode: 0xA1
/// Original: bochs/cpu/data_xfer32.cc:96-101 MOV_EAXOd
/// Load EAX from memory at direct address (seg:offset)
pub fn MOV_EAXOd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let offset = instr.id() as u64;
    let ds_base = unsafe { cpu.sregs[crate::cpu::decoder::BxSegregs::Ds as usize].cache.u.segment.base };
    let addr = ds_base.wrapping_add(offset);
    let value = cpu.mem_read_dword(addr);
    cpu.set_eax(value);

    tracing::trace!("MOV EAX, [DS:{:#x}]: {:#x}", offset, value);
    Ok(())
}

/// MOV_OdEAX: MOV moffs32, EAX
/// Opcode: 0xA3
/// Original: bochs/cpu/data_xfer32.cc:103-108 MOV_OdEAX
/// Store EAX to memory at direct address (seg:offset)
pub fn MOV_OdEAX<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let offset = instr.id() as u64;
    let ds_base = unsafe { cpu.sregs[crate::cpu::decoder::BxSegregs::Ds as usize].cache.u.segment.base };
    let addr = ds_base.wrapping_add(offset);
    cpu.mem_write_dword(addr, cpu.eax());

    tracing::trace!("MOV [DS:{:#x}], EAX: {:#x}", offset, cpu.eax());
    Ok(())
}
