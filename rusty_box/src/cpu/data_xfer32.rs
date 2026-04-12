#![allow(non_snake_case, dead_code)]

// 32-bit data transfer operations: MOV, etc.
// Mirrors Bochs cpp/cpu/data_xfer32.cc

use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// MOV_GdEd_R: MOV r32, r/m32 (register form)
/// Opcode: 0x8B, ModRM: r32, r/m32 (register)
/// operands.dst = destination register
/// operands.src1 = source register
pub fn MOV_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_idx = instr.dst() as usize;
    let src_idx = instr.src1() as usize;

    let val = cpu.get_gpr32(src_idx);
    cpu.set_gpr32(dst_idx, val);
}

/// MOV_EdGd_R: MOV r/m32, r32 (register form)
/// Opcode: 0x89, ModRM: r/m32, r32 (register)
/// Decoder swaps for 16/32-bit store: operands.dst = rm (DESTINATION), operands.src1 = nnn (SOURCE)
pub fn MOV_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let val = cpu.get_gpr32(instr.src1() as usize); // nnn = source
    cpu.set_gpr32(instr.dst() as usize, val); // rm = destination
}

/// MOV_EdId_R: MOV r/m32, imm32 (register form)
/// Opcode: 0xC7, ModRM: r/m32, imm32 (register)
/// operands.dst = destination register
/// Immediate value stored in operand_data.Id
pub fn MOV_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_idx = instr.dst() as usize;
    let imm: u32 = instr.id();

    cpu.set_gpr32(dst_idx, imm);
}

/// MOV_GdEd_M: MOV r32, r/m32 (memory form)
/// Opcode: 0x8B, ModRM: r32, r/m32 (memory)
/// Bochs: MOV32_GdEdM
pub fn MOV_GdEd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let val = cpu.v_read_dword(seg, eaddr)?;
    let dst_idx = instr.dst() as usize;
    cpu.set_gpr32(dst_idx, val);
    Ok(())
}

/// MOV_EdGd_M: MOV r/m32, r32 (memory form)
/// Opcode: 0x89, ModRM: r/m32, r32 (memory)
/// Bochs: MOV32_EdGdM
/// Decoder swaps for 16/32-bit store: operands.src1 (src()) = nnn = SOURCE register
pub fn MOV_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let val = cpu.get_gpr32(instr.src() as usize);
    cpu.v_write_dword(seg, eaddr, val)?;
    Ok(())
}

/// MOV_EdId_M: MOV r/m32, imm32 (memory form)
/// Opcode: 0xC7, ModRM: r/m32, imm32 (memory)
/// Bochs: MOV_EdIdM
pub fn MOV_EdId_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let imm = instr.id();
    cpu.v_write_dword(seg, eaddr, imm)?;
    Ok(())
}

/// MOVZX_GdEb_M: MOVZX r32, r/m8 (memory form)
/// Bochs: MOVZX_GdEbM
pub fn MOVZX_GdEb_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let val = cpu.v_read_byte(seg, eaddr)?;
    let dst_reg = instr.dst() as usize;
    cpu.set_gpr32(dst_reg, val as u32);
    Ok(())
}

/// MOVZX_GdEw_M: MOVZX r32, r/m16 (memory form)
/// Bochs: MOVZX_GdEwM
pub fn MOVZX_GdEw_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let val = cpu.v_read_word(seg, eaddr)?;
    let dst_reg = instr.dst() as usize;
    cpu.set_gpr32(dst_reg, val as u32);
    Ok(())
}

// =========================================================================
// Unified handlers: dispatch R/M based on instr.mod_c0()
// =========================================================================

/// MOV r32, r/m32 - unified (Bochs: MOV_GdEdR / MOV_GdEdM)
pub fn MOV_GdEd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        MOV_GdEd_R(cpu, instr);
        Ok(())
    } else {
        MOV_GdEd_M(cpu, instr)
    }
}

/// MOV r/m32, r32 - unified (Bochs: MOV_EdGdR / MOV_EdGdM)
pub fn MOV_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        MOV_EdGd_R(cpu, instr);
        Ok(())
    } else {
        MOV_EdGd_M(cpu, instr)
    }
}

/// MOV r/m32, imm32 - unified (Bochs: MOV_EdIdR / MOV_EdIdM)
pub fn MOV_EdId<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        MOV_EdId_R(cpu, instr);
        Ok(())
    } else {
        MOV_EdId_M(cpu, instr)
    }
}

/// MOVZX r32, r/m8 - unified (Bochs: MOVZX_GdEbR / MOVZX_GdEbM)
pub fn MOVZX_GdEb_unified<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        MOVZX_GdEb(cpu, instr);
        Ok(())
    } else {
        MOVZX_GdEb_M(cpu, instr)
    }
}

/// MOVZX r32, r/m16 - unified (Bochs: MOVZX_GdEwR / MOVZX_GdEwM)
pub fn MOVZX_GdEw_unified<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        MOVZX_GdEw(cpu, instr);
        Ok(())
    } else {
        MOVZX_GdEw_M(cpu, instr)
    }
}

/// MOV_EAX_Id: MOV EAX, imm32 (register direct)
/// Opcodes: 0xB8-0xBF (0xB8 + register index)
/// operands.dst = register index
pub fn MOV_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_idx = instr.dst() as usize;
    let imm: u32 = instr.id();

    cpu.set_gpr32(dst_idx, imm);
}

/// MOVZX_GdEb: MOVZX r32, r/m8
/// Opcode: 0x0F 0xB6, ModRM: r32, r/m8
/// Original: bochs/cpu/ MOVZX_GdEbM/MOVZX_GdEbR
/// Zero extend byte operand into dword destination
pub fn MOVZX_GdEb<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_reg = instr.dst() as usize;
    let src_reg = instr.src1() as usize;

    // Read 8-bit operand — REX-aware (SPL/BPL/SIL/DIL when REX present)
    let op2_8 = cpu.read_8bit_regx(src_reg, instr.extend8bit_l());

    // Zero extend byte op2 into dword op1
    cpu.set_gpr32(dst_reg, op2_8 as u32);
}

/// MOVZX_GdEw: MOVZX r32, r/m16
/// Opcode: 0x0F 0xB7, ModRM: r32, r/m16
/// Original: bochs/cpu/ MOVZX_GdEwM/MOVZX_GdEwR
/// Zero extend word operand into dword destination
pub fn MOVZX_GdEw<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_reg = instr.dst() as usize;
    let src_reg = instr.src1() as usize;

    // Read 16-bit operand (handles both memory and register based on decoder metadata)
    let op2_16 = cpu.get_gpr16(src_reg);

    // Zero extend word op2 into dword op1
    cpu.set_gpr32(dst_reg, op2_16 as u32);
}

/// MOV_EAXOd: MOV EAX, moffs32
/// Opcode: 0xA1
/// Original: bochs/cpu/ MOV_EAXOd
/// Load EAX from memory at direct address (seg:offset)
pub fn MOV_EAXOd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let seg = crate::cpu::decoder::BxSegregs::from(instr.seg());
    let offset = instr.id();
    let value = cpu.v_read_dword(seg, offset)?;
    cpu.set_eax(value);
    Ok(())
}

/// MOV_OdEAX: MOV moffs32, EAX
/// Opcode: 0xA3
/// Original: bochs/cpu/ MOV_OdEAX
/// Store EAX to memory at direct address (seg:offset)
pub fn MOV_OdEAX<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let seg = crate::cpu::decoder::BxSegregs::from(instr.seg());
    let offset = instr.id();
    let eax_val = cpu.eax();
    cpu.v_write_dword(seg, offset, eax_val)?;
    Ok(())
}
