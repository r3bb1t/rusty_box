// 16-bit data transfer operations: MOV, etc.
// Mirrors Bochs cpp/cpu/data_xfer16.cc

use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::cpu::decoder::{Instruction, BxSegregs};

/// MOV_AXOd: MOV AX, moffs16 - Load AX from memory
/// Opcode: 0xA1 (16-bit operand size)
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_AXOd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) -> Result<(), crate::cpu::CpuError> {
    let seg = BxSegregs::from(instr.seg());
    let offset = instr.id();
    let val = cpu.read_virtual_word(seg, offset)?;
    cpu.set_ax(val);
    Ok(())
}

/// MOV_OdAX: MOV moffs16, AX - Store AX to memory
/// Opcode: 0xA3 (16-bit operand size)
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_OdAX<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) -> Result<(), crate::cpu::CpuError> {
    let seg = BxSegregs::from(instr.seg());
    let offset = instr.id();
    cpu.write_virtual_word(seg, offset, cpu.ax())?;
    Ok(())
}
