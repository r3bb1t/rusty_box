// 8-bit data transfer operations: MOV, etc.
// Mirrors Bochs cpp/cpu/data_xfer8.cc

use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::cpu::decoder::{BxInstructionGenerated, BxSegregs};

/// MOV_ALOd: MOV AL, moffs8 - Load AL from memory
/// Opcode: 0xA0
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_ALOd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let offset = instr.id() as u64;
    let ds_base = unsafe { cpu.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
    let addr = ds_base.wrapping_add(offset);
    let val = cpu.mem_read_byte(addr);
    cpu.set_al(val);
    Ok(())
}

/// MOV_OdAL: MOV moffs8, AL - Store AL to memory
/// Opcode: 0xA2
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_OdAL<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let offset = instr.id() as u64;
    let ds_base = unsafe { cpu.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
    let addr = ds_base.wrapping_add(offset);
    cpu.mem_write_byte(addr, cpu.al());
    Ok(())
}

/// MOV_GbEbM: MOV r8, r/m8 - Load register from memory
/// Opcode: 0x8A (memory form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc:43 MOV_GbEbM
pub fn MOV_GbEbM<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve effective address (matching BX_CPU_RESOLVE_ADDR)
    let eaddr = cpu.resolve_addr32(instr);

    // Get segment (with override support)
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };

    // Read byte from virtual memory (matching read_virtual_byte)
    let val = cpu.read_virtual_byte(seg, eaddr);

    // Write to destination register (matching BX_WRITE_8BIT_REGx)
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), val);

    Ok(())
}

/// MOV_GbEbR: MOV r8, r8 - Register to register (opcode 0x8A, register form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc:53 MOV_GbEbR
pub fn MOV_GbEbR<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op2 = cpu.read_8bit_regx(instr.src() as usize, instr.extend8bit_l());
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), op2);
    Ok(())
}

/// MOV_EbGbM: MOV r/m8, r8 - Store register to memory
/// Opcode: 0x88 (memory form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc:61 (MOV_EbGbM)
pub fn MOV_EbGbM<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    // Resolve effective address
    let eaddr = cpu.resolve_addr32(instr);

    // Get segment (with override support)
    let seg = unsafe { core::mem::transmute::<u8, BxSegregs>(instr.seg()) };

    // Read from source register
    let val = cpu.read_8bit_regx(instr.src() as usize, instr.extend8bit_l());

    // Write byte to virtual memory
    cpu.write_virtual_byte(seg, eaddr, val);

    Ok(())
}

/// MOV_EbGbR: MOV r8, r8 - Register to register (opcode 0x88, register form)
/// Mirrors Bochs cpp/cpu/data_xfer8.cc:69 MOV_EbGbR
pub fn MOV_EbGbR<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let op2 = cpu.read_8bit_regx(instr.src() as usize, instr.extend8bit_l());
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), op2);
    Ok(())
}
