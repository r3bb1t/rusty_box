#![allow(non_snake_case)]

// BCD (Binary Coded Decimal) instructions: DAA, DAS, AAA, AAS, AAM, AAD
// Mirrors Bochs cpp/cpu/bcd.cc

use crate::cpu::decoder::Instruction;
use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// AAA: ASCII Adjust After Addition
/// Opcode: 0x37
/// Matches Bochs bcd.cc BX_CPU_C::AAA
pub fn AAA<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    _instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let mut tmp_cf = false;
    let mut tmp_af = false;

    if ((cpu.al() & 0x0F) > 9) || cpu.get_af() {
        let ax = cpu.ax().wrapping_add(0x106);
        cpu.set_ax(ax);
        tmp_af = true;
        tmp_cf = true;
    }

    cpu.set_al(cpu.al() & 0x0F);

    cpu.update_flags_logic8(cpu.al());
    cpu.set_cf(tmp_cf);
    cpu.set_af(tmp_af);

    Ok(())
}

/// AAS: ASCII Adjust After Subtraction
/// Opcode: 0x3F
/// Matches Bochs bcd.cc BX_CPU_C::AAS
pub fn AAS<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    _instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let mut tmp_cf = false;
    let mut tmp_af = false;

    if ((cpu.al() & 0x0F) > 0x09) || cpu.get_af() {
        let ax = cpu.ax().wrapping_sub(0x106);
        cpu.set_ax(ax);
        tmp_af = true;
        tmp_cf = true;
    }

    cpu.set_al(cpu.al() & 0x0F);

    cpu.update_flags_logic8(cpu.al());
    cpu.set_cf(tmp_cf);
    cpu.set_af(tmp_af);

    Ok(())
}

/// AAM: ASCII Adjust AX After Multiply
/// Opcode: 0xD4 imm8
/// Matches Bochs bcd.cc BX_CPU_C::AAM
pub fn AAM<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let imm8 = instr.ib();
    if imm8 == 0 {
        return cpu.exception(crate::cpu::cpu::Exception::De, 0);
    }

    let al = cpu.al();
    cpu.set_ah(al / imm8);
    cpu.set_al(al % imm8);

    cpu.update_flags_logic8(cpu.al());

    Ok(())
}

/// AAD: ASCII Adjust AX Before Division
/// Opcode: 0xD5 imm8
/// Matches Bochs bcd.cc BX_CPU_C::AAD
pub fn AAD<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let tmp = (cpu.ah() as u16)
        .wrapping_mul(instr.ib() as u16)
        .wrapping_add(cpu.al() as u16);
    cpu.set_ax(tmp & 0xFF);

    cpu.update_flags_logic8(cpu.al());

    Ok(())
}

/// DAA: Decimal Adjust AL after Addition
/// Opcode: 0x27
/// Matches Bochs bcd.cc BX_CPU_C::DAA
pub fn DAA<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    _instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let tmp_al = cpu.al();
    let original_cf = cpu.get_cf();
    let mut tmp_cf = false;
    let mut tmp_af = false;

    if ((tmp_al & 0x0F) > 0x09) || cpu.get_af() {
        tmp_cf = (cpu.al() > 0xF9) || original_cf;
        cpu.set_al(cpu.al().wrapping_add(0x06));
        tmp_af = true;
    }

    if (tmp_al > 0x99) || original_cf {
        cpu.set_al(cpu.al().wrapping_add(0x60));
        tmp_cf = true;
    }

    cpu.update_flags_logic8(cpu.al());
    cpu.set_cf(tmp_cf);
    cpu.set_af(tmp_af);

    Ok(())
}

/// DAS: Decimal Adjust AL after Subtraction
/// Opcode: 0x2F
/// Matches Bochs bcd.cc BX_CPU_C::DAS
pub fn DAS<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<I, T>,
    _instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let tmp_al = cpu.al();
    let original_cf = cpu.get_cf();
    let mut tmp_cf = false;
    let mut tmp_af = false;

    if ((tmp_al & 0x0F) > 0x09) || cpu.get_af() {
        tmp_cf = (cpu.al() < 0x06) || original_cf;
        cpu.set_al(cpu.al().wrapping_sub(0x06));
        tmp_af = true;
    }

    if (tmp_al > 0x99) || original_cf {
        cpu.set_al(cpu.al().wrapping_sub(0x60));
        tmp_cf = true;
    }

    cpu.update_flags_logic8(cpu.al());
    cpu.set_cf(tmp_cf);
    cpu.set_af(tmp_af);

    Ok(())
}
