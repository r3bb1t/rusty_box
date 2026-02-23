// 16-bit data transfer operations: MOV, etc.
// Mirrors Bochs cpp/cpu/data_xfer16.cc

use crate::cpu::{BxCpuC, BxCpuIdTrait};
use crate::cpu::decoder::{BxInstructionGenerated, BxSegregs};

/// MOV_AXOd: MOV AX, moffs16 - Load AX from memory
/// Opcode: 0xA1
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_AXOd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let offset = instr.id() as u64;
    let ds_base = unsafe { cpu.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
    let addr = ds_base.wrapping_add(offset);
    let val = cpu.mem_read_word(addr);
    cpu.set_ax(val);
    // Watchpoint: BDA timer tick counter
    if addr == 0x046C || offset == 0x046C {
        tracing::warn!("WP-READ16: MOV AX,[{:#x}] (offset={:#x}, DS.base={:#x}) = {:#06x}, RIP={:#x}",
            addr, offset, ds_base, val, cpu.rip());
    }
    Ok(())
}

/// MOV_OdAX: MOV moffs16, AX - Store AX to memory
/// Opcode: 0xA3
/// Segment: DS (default) or override prefix
/// Offset: 16-bit or 32-bit immediate offset (i.Id())
pub fn MOV_OdAX<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let offset = instr.id() as u64;
    let ds_base = unsafe { cpu.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
    let addr = ds_base.wrapping_add(offset);
    cpu.mem_write_word(addr, cpu.ax());
    // Watchpoint: BDA timer tick counter
    if addr == 0x046C || offset == 0x046C {
        tracing::warn!("WP-WRITE16: MOV [{:#x}],AX (offset={:#x}, DS.base={:#x}) = {:#06x}, RIP={:#x}",
            addr, offset, ds_base, cpu.ax(), cpu.rip());
    }
    Ok(())
}
