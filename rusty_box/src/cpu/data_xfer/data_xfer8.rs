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
