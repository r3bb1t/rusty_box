#![allow(non_snake_case, dead_code)]

// 8-bit data transfer operations: MOV, etc.
// Mirrors Bochs cpp/cpu/data_xfer8.cc

use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// MOV_ALOd: MOV AL, moffs8 - Load AL from memory
/// Opcode: 0xA0
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_ALOd<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let seg = BxSegregs::from(instr.seg());
    let offset = instr.id();
    let val = cpu.v_read_byte(seg, offset)?;
    cpu.set_al(val);
    Ok(())
}

/// MOV_OdAL: MOV moffs8, AL - Store AL to memory
/// Opcode: 0xA2
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_OdAL<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let seg = BxSegregs::from(instr.seg());
    let offset = instr.id();
    cpu.v_write_byte(seg, offset, cpu.al())?;
    Ok(())
}

/// MOV_GbEbM: MOV r8, r/m8 - Load register from memory
/// Opcode: 0x8A (memory form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc MOV_GbEbM
pub fn MOV_GbEbM<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    // Resolve effective address (matching BX_CPU_RESOLVE_ADDR)
    let eaddr = cpu.resolve_addr(instr);

    // Get segment (with override support)
    let seg = BxSegregs::from(instr.seg());

    // Read byte from virtual memory (matching read_virtual_byte)
    let val = cpu.v_read_byte(seg, eaddr)?;

    // Write to destination register (matching BX_WRITE_8BIT_REGx)
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), val);

    Ok(())
}

/// MOV_GbEbR: MOV r8, r8 - Register to register (opcode 0x8A, register form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc MOV_GbEbR
pub fn MOV_GbEbR<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op2 = cpu.read_8bit_regx(instr.src() as usize, instr.extend8bit_l());
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), op2);
    Ok(())
}

/// MOV_EbGbM: MOV r/m8, r8 - Store register to memory
/// Opcode: 0x88 (memory form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc (MOV_EbGbM)
pub fn MOV_EbGbM<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    // Resolve effective address
    let eaddr = cpu.resolve_addr(instr);

    // Get segment (with override support)
    let seg = BxSegregs::from(instr.seg());

    // Read from source register - use dst() because the decoder stores
    // the ModRM reg field in operands.dst (dst), regardless of direction.
    // For opcode 0x88 (MOV r/m8, r8), reg is the SOURCE register.
    let val = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());

    // Write byte to virtual memory
    cpu.v_write_byte(seg, eaddr, val)?;

    Ok(())
}

/// MOV_EbGbR: MOV r/m8, r8 - Register to register (opcode 0x88, register form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc MOV_EbGbR
/// Note: decoder always stores reg->operands.dst(dst), rm->operands.src1(src).
/// For opcode 0x88, reg=source and rm=destination, so we swap access.
pub fn MOV_EbGbR<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    cpu.write_8bit_regx(instr.src() as usize, instr.extend8bit_l(), op2);
    Ok(())
}
