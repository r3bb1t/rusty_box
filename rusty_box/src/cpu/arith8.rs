// 8-bit arithmetic operations: ADD, SUB, etc.
// Mirrors Bochs cpp/cpu/arith8.cc

use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::eflags::EFlags;
use crate::cpu::{BxCpuC, BxCpuIdTrait};

// Helper methods are defined in logical8.rs and data_xfer_ext.rs
// Free functions below use cpu.resolve_addr32(), cpu.read_8bit_regx(), etc.
// which call the public methods from those modules

/// ADD_EbGbM: ADD r/m8, r8 (memory form)
/// Opcode: 0x00, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::ADD_EbGbM
pub fn ADD_EbGbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg field = source for store-direction
    let sum = op1.wrapping_add(op2);

    cpu.write_rmw_linear_byte(sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADD_GbEbR: ADD r8, r/m8 (register form)
/// Opcode: 0x02, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::ADD_GbEbR
pub fn ADD_GbEbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let sum = op1.wrapping_add(op2);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADD_GbEbM: ADD r8, r/m8 (memory form)
/// Opcode: 0x02, ModRM: r8, r/m8 (memory)
/// Matches BX_CPU_C::ADD_GbEbM
pub fn ADD_GbEbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_virtual_byte(seg, eaddr)?;
    let sum = op1.wrapping_add(op2);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADD_EbGb: ADD r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn ADD_EbGb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: ADD r/m8, r8
        // Opcode 0x00: reg (dst()) = SOURCE, rm (src1()) = DESTINATION
        let op1 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l()); // rm = destination
        let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg = source
        let sum = op1.wrapping_add(op2);

        cpu.write_8bit_regx(instr.src1() as usize, instr.extend8bit_l(), sum); // write to rm
        cpu.update_flags_add8(op1, op2, sum);

        Ok(())
    } else {
        // Memory form
        ADD_EbGbM(cpu, instr)
    }
}

/// ADD_GbEb: ADD r8, r/m8
/// Dispatches to memory or register form based on ModRM
pub fn ADD_GbEb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form
        ADD_GbEbR(cpu, instr)
    } else {
        // Memory form
        ADD_GbEbM(cpu, instr)
    }
}

/// SUB_EbGbM: SUB r/m8, r8 (memory form)
/// Opcode: 0x28, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::SUB_EbGbM
pub fn SUB_EbGbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg field = source for store-direction
    let diff = op1.wrapping_sub(op2);

    cpu.write_rmw_linear_byte(diff);
    cpu.update_flags_sub8(op1, op2, diff);

    Ok(())
}

/// SUB_GbEbR: SUB r8, r/m8 (register form)
/// Opcode: 0x2A, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::SUB_GbEbR
pub fn SUB_GbEbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let diff = op1.wrapping_sub(op2);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), diff);
    cpu.update_flags_sub8(op1, op2, diff);

    Ok(())
}

/// SUB_GbEbM: SUB r8, r/m8 (memory form)
/// Opcode: 0x2A, ModRM: r8, r/m8 (memory)
/// Matches BX_CPU_C::SUB_GbEbM
pub fn SUB_GbEbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_virtual_byte(seg, eaddr)?;
    let diff = op1.wrapping_sub(op2);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), diff);
    cpu.update_flags_sub8(op1, op2, diff);

    Ok(())
}

/// SUB_EbGb: SUB r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn SUB_EbGb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: SUB r/m8, r8
        // Opcode 0x28: reg (dst()) = SOURCE, rm (src1()) = DESTINATION
        let op1 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l()); // rm = destination
        let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg = source
        let diff = op1.wrapping_sub(op2);

        cpu.write_8bit_regx(instr.src1() as usize, instr.extend8bit_l(), diff); // write to rm
        cpu.update_flags_sub8(op1, op2, diff);

        Ok(())
    } else {
        // Memory form
        SUB_EbGbM(cpu, instr)
    }
}

/// SUB_GbEb: SUB r8, r/m8
/// Dispatches to memory or register form based on ModRM
pub fn SUB_GbEb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form
        SUB_GbEbR(cpu, instr)
    } else {
        // Memory form
        SUB_GbEbM(cpu, instr)
    }
}

/// AND_EbGbM: AND r/m8, r8 (memory form)
/// Opcode: 0x20, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::AND_EbGbM
pub fn AND_EbGbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg field = source
    let result = op1 & op2;

    cpu.write_rmw_linear_byte(result);
    // Update flags for logical operation (AND)
    let sf = (result & 0x80) != 0;
    let zf = result == 0;
    let pf = result.count_ones() % 2 == 0;
    const MASK: EFlags = EFlags::CF
        .union(EFlags::PF)
        .union(EFlags::ZF)
        .union(EFlags::SF)
        .union(EFlags::OF);
    cpu.eflags.remove(MASK);
    if pf {
        cpu.eflags.insert(EFlags::PF);
    }
    if zf {
        cpu.eflags.insert(EFlags::ZF);
    }
    if sf {
        cpu.eflags.insert(EFlags::SF);
    }

    Ok(())
}

/// AND_GbEbR: AND r8, r8 (register form)
/// Opcode: 0x20, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::AND_GbEbR
pub fn AND_GbEbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let result = op1 & op2;

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), result);
    // Update flags for logical operation (AND)
    let sf = (result & 0x80) != 0;
    let zf = result == 0;
    let pf = result.count_ones() % 2 == 0;
    const MASK: EFlags = EFlags::CF
        .union(EFlags::PF)
        .union(EFlags::ZF)
        .union(EFlags::SF)
        .union(EFlags::OF);
    cpu.eflags.remove(MASK);
    if pf {
        cpu.eflags.insert(EFlags::PF);
    }
    if zf {
        cpu.eflags.insert(EFlags::ZF);
    }
    if sf {
        cpu.eflags.insert(EFlags::SF);
    }

    Ok(())
}

/// AND_EbGb: AND r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn AND_EbGb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: AND r/m8, r8
        // Opcode 0x20: reg (dst()) = SOURCE, rm (src1()) = DESTINATION
        let op1 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l()); // rm = destination
        let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg = source
        let result = op1 & op2;

        cpu.write_8bit_regx(instr.src1() as usize, instr.extend8bit_l(), result); // write to rm
                                                                                  // Update flags for logical operation (AND)
        let sf = (result & 0x80) != 0;
        let zf = result == 0;
        let pf = result.count_ones() % 2 == 0;
        const MASK: EFlags = EFlags::CF
            .union(EFlags::PF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        cpu.eflags.remove(MASK);
        if pf {
            cpu.eflags.insert(EFlags::PF);
        }
        if zf {
            cpu.eflags.insert(EFlags::ZF);
        }
        if sf {
            cpu.eflags.insert(EFlags::SF);
        }

        Ok(())
    } else {
        // Memory form
        AND_EbGbM(cpu, instr)
    }
}

/// ADC_EbGbM: ADC r/m8, r8 (memory form)
/// Opcode: 0x10, ModRM: r/m8, r8 (memory)
/// Matches BX_CPU_C::ADC_EbGbM
pub fn ADC_EbGbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg field = source
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);

    cpu.write_rmw_linear_byte(sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADC_GbEbR: ADC r8, r8 (register form)
/// Opcode: 0x10, ModRM: r8, r/m8 (register)
/// Matches BX_CPU_C::ADC_GbEbR
pub fn ADC_GbEbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADC_EbGb: ADC r/m8, r8
/// Dispatches to memory or register form based on ModRM
pub fn ADC_EbGb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        // Register form: ADC r/m8, r8
        // Opcode 0x10: reg (dst()) = SOURCE, rm (src1()) = DESTINATION
        let op1 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l()); // rm = destination
        let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg = source
        let cf = cpu.get_cf() as u8;
        let sum = op1.wrapping_add(op2).wrapping_add(cf);

        cpu.write_8bit_regx(instr.src1() as usize, instr.extend8bit_l(), sum); // write to rm
        cpu.update_flags_add8(op1, op2, sum);

        Ok(())
    } else {
        // Memory form
        ADC_EbGbM(cpu, instr)
    }
}

/// ADC_GbEbM: ADC r8, r/m8 (memory form)
/// Opcode: 0x12, ModRM: r8, r/m8 (memory)
/// Matches BX_CPU_C::ADC_GbEbM
pub fn ADC_GbEbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_virtual_byte(seg, eaddr)?;
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADC_GbEb: ADC r8, r/m8
/// Dispatches to memory or register form based on ModRM
pub fn ADC_GbEb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_GbEbR(cpu, instr)
    } else {
        ADC_GbEbM(cpu, instr)
    }
}

/// ADD_EbIbR: ADD r/m8, imm8 (register form)
/// Opcode: 0x80/0 or 0x83/0 (8-bit)
/// Matches BX_CPU_C::ADD_EbIbR
pub fn ADD_EbIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = instr.ib();
    let sum = op1.wrapping_add(op2);

    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// ADD_EbIbM: ADD r/m8, imm8 (memory form)
pub fn ADD_EbIbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = instr.ib();
    let sum = op1.wrapping_add(op2);
    cpu.write_rmw_linear_byte(sum);
    cpu.update_flags_add8(op1, op2, sum);
    Ok(())
}

/// ADD_EbIb: ADD r/m8, imm8 - unified dispatch
pub fn ADD_EbIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_EbIbR(cpu, instr)
    } else {
        ADD_EbIbM(cpu, instr)
    }
}

/// ADD_ALIb: ADD AL, imm8
/// Dedicated handler for opcode 0x04 - accumulator-immediate form
/// Must hardcode AL (register 0) because the decoder sets dst from opcode
/// low bits (b1 & 7 = 4 for opcode 0x04), which would be AH, not AL.
pub fn ADD_ALIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.al();
    let op2 = instr.ib();
    let sum = op1.wrapping_add(op2);

    cpu.set_al(sum);
    cpu.update_flags_add8(op1, op2, sum);

    Ok(())
}

/// SUB_EbIbR: SUB r/m8, imm8 (register form)
pub fn SUB_EbIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = instr.ib();
    let diff = op1.wrapping_sub(op2);
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SUB_EbIbM: SUB r/m8, imm8 (memory form)
pub fn SUB_EbIbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = instr.ib();
    let diff = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_byte(diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SUB_EbIb: SUB r/m8, imm8 - unified dispatch
pub fn SUB_EbIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_EbIbR(cpu, instr)
    } else {
        SUB_EbIbM(cpu, instr)
    }
}

/// ADC_EbIbR: ADC r/m8, imm8 (register form)
pub fn ADC_EbIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = instr.ib();
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), sum);
    cpu.update_flags_add8(op1, op2, sum);
    Ok(())
}

/// ADC_EbIbM: ADC r/m8, imm8 (memory form)
pub fn ADC_EbIbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = instr.ib();
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_byte(sum);
    cpu.update_flags_add8(op1, op2, sum);
    Ok(())
}

/// ADC_EbIb: ADC r/m8, imm8 - unified dispatch
pub fn ADC_EbIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EbIbR(cpu, instr)
    } else {
        ADC_EbIbM(cpu, instr)
    }
}

/// SBB_EbIbR: SBB r/m8, imm8 (register form)
pub fn SBB_EbIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = instr.ib();
    let cf = cpu.get_cf() as u8;
    let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SBB_EbIbM: SBB r/m8, imm8 (memory form)
pub fn SBB_EbIbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = instr.ib();
    let cf = cpu.get_cf() as u8;
    let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_byte(diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SBB_EbIb: SBB r/m8, imm8 - unified dispatch
pub fn SBB_EbIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EbIbR(cpu, instr)
    } else {
        SBB_EbIbM(cpu, instr)
    }
}

/// ADC_ALIb: ADC AL, imm8
/// Dedicated handler for opcode 0x14 - accumulator-immediate form
/// Must hardcode AL (register 0) because the decoder sets dst from opcode
/// low bits (b1 & 7 = 4 for opcode 0x14), which would be AH, not AL.
pub fn ADC_ALIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.al();
    let op2 = instr.ib();
    let cf = cpu.get_cf() as u8;
    let sum = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.set_al(sum);
    cpu.update_flags_add8(op1, op2, sum);
    Ok(())
}

/// SBB_ALIb: SBB AL, imm8
/// Dedicated handler for opcode 0x1C - accumulator-immediate form
/// Must hardcode AL (register 0) because the decoder sets dst from opcode
/// low bits (b1 & 7 = 4 for opcode 0x1C), which would be AH, not AL.
pub fn SBB_ALIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.al();
    let op2 = instr.ib();
    let cf = cpu.get_cf() as u8;
    let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_al(diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SBB_EbGbM: SBB r/m8, r8 (memory form)
pub fn SBB_EbGbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let cf = cpu.get_cf() as u8;
    let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_byte(diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SBB_EbGb: SBB r/m8, r8 - unified dispatch
pub fn SBB_EbGb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        let op1 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
        let op2 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let cf = cpu.get_cf() as u8;
        let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
        cpu.write_8bit_regx(instr.src1() as usize, instr.extend8bit_l(), diff);
        cpu.update_flags_sub8(op1, op2, diff);
        Ok(())
    } else {
        SBB_EbGbM(cpu, instr)
    }
}

/// SBB_GbEbR: SBB r8, r8 (register form)
pub fn SBB_GbEbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_8bit_regx(instr.src1() as usize, instr.extend8bit_l());
    let cf = cpu.get_cf() as u8;
    let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SBB_GbEbM: SBB r8, r/m8 (memory form)
pub fn SBB_GbEbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
    let op2 = cpu.read_virtual_byte(seg, eaddr)?;
    let cf = cpu.get_cf() as u8;
    let diff = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_8bit_regx(instr.dst() as usize, instr.extend8bit_l(), diff);
    cpu.update_flags_sub8(op1, op2, diff);
    Ok(())
}

/// SBB_GbEb: SBB r8, r/m8 - unified dispatch
pub fn SBB_GbEb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_GbEbR(cpu, instr)
    } else {
        SBB_GbEbM(cpu, instr)
    }
}

/// INC_Eb: INC r/m8 (register form)
/// Opcode: 0xFE/0
/// Original: bochs/cpu/arith8.cc INC_EbR
/// Increment 8-bit register by 1
pub fn INC_Eb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.read_8bit_regx(dst, instr.extend8bit_l());
    let result = op1.wrapping_add(1);
    cpu.write_8bit_regx(dst, instr.extend8bit_l(), result);

    // INC affects OF, SF, ZF, AF, PF but NOT CF
    let zf = result == 0;
    let sf = (result & 0x80) != 0;
    let of = result == 0x80; // Overflow if we wrapped from 0x7F to 0x80
    let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
    let pf = result.count_ones() % 2 == 0;

    // Update all flags except CF (bit 0)
    const MASK: EFlags = EFlags::PF
        .union(EFlags::AF)
        .union(EFlags::ZF)
        .union(EFlags::SF)
        .union(EFlags::OF);
    cpu.eflags.remove(MASK);
    if pf {
        cpu.eflags.insert(EFlags::PF);
    }
    if af {
        cpu.eflags.insert(EFlags::AF);
    }
    if zf {
        cpu.eflags.insert(EFlags::ZF);
    }
    if sf {
        cpu.eflags.insert(EFlags::SF);
    }
    if of {
        cpu.eflags.insert(EFlags::OF);
    }

    Ok(())
}

/// DEC_Eb: DEC r/m8 (register form)
/// Opcode: 0xFE/1
/// Original: bochs/cpu/arith8.cc DEC_EbR
/// Decrement 8-bit register by 1
pub fn DEC_Eb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.read_8bit_regx(dst, instr.extend8bit_l());
    let result = op1.wrapping_sub(1);
    cpu.write_8bit_regx(dst, instr.extend8bit_l(), result);

    // DEC affects OF, SF, ZF, AF, PF but NOT CF
    let zf = result == 0;
    let sf = (result & 0x80) != 0;
    let of = result == 0x7F; // Overflow if we wrapped from 0x80 to 0x7F
    let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
    let pf = result.count_ones() % 2 == 0;

    const MASK: EFlags = EFlags::PF
        .union(EFlags::AF)
        .union(EFlags::ZF)
        .union(EFlags::SF)
        .union(EFlags::OF);
    cpu.eflags.remove(MASK);
    if pf {
        cpu.eflags.insert(EFlags::PF);
    }
    if af {
        cpu.eflags.insert(EFlags::AF);
    }
    if zf {
        cpu.eflags.insert(EFlags::ZF);
    }
    if sf {
        cpu.eflags.insert(EFlags::SF);
    }
    if of {
        cpu.eflags.insert(EFlags::OF);
    }

    Ok(())
}

/// INC_EbM: INC r/m8 (memory form) — matches Bochs INC_EbM
pub fn INC_EbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let result = op1.wrapping_add(1);
    cpu.write_rmw_linear_byte(result);

    let zf = result == 0;
    let sf = (result & 0x80) != 0;
    let of = result == 0x80;
    let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
    let pf = result.count_ones() % 2 == 0;

    const MASK: EFlags = EFlags::PF
        .union(EFlags::AF)
        .union(EFlags::ZF)
        .union(EFlags::SF)
        .union(EFlags::OF);
    cpu.eflags.remove(MASK);
    if pf {
        cpu.eflags.insert(EFlags::PF);
    }
    if af {
        cpu.eflags.insert(EFlags::AF);
    }
    if zf {
        cpu.eflags.insert(EFlags::ZF);
    }
    if sf {
        cpu.eflags.insert(EFlags::SF);
    }
    if of {
        cpu.eflags.insert(EFlags::OF);
    }

    Ok(())
}

/// DEC_EbM: DEC r/m8 (memory form) — matches Bochs DEC_EbM
pub fn DEC_EbM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_rmw_virtual_byte(seg, eaddr)?;
    let result = op1.wrapping_sub(1);
    cpu.write_rmw_linear_byte(result);

    let zf = result == 0;
    let sf = (result & 0x80) != 0;
    let of = result == 0x7F;
    let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
    let pf = result.count_ones() % 2 == 0;

    const MASK: EFlags = EFlags::PF
        .union(EFlags::AF)
        .union(EFlags::ZF)
        .union(EFlags::SF)
        .union(EFlags::OF);
    cpu.eflags.remove(MASK);
    if pf {
        cpu.eflags.insert(EFlags::PF);
    }
    if af {
        cpu.eflags.insert(EFlags::AF);
    }
    if zf {
        cpu.eflags.insert(EFlags::ZF);
    }
    if sf {
        cpu.eflags.insert(EFlags::SF);
    }
    if of {
        cpu.eflags.insert(EFlags::OF);
    }

    Ok(())
}

/// INC r/m8 - Unified dispatch based on mod_c0()
pub fn inc_eb_dispatch<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        INC_Eb(cpu, instr)
    } else {
        INC_EbM(cpu, instr)
    }
}

/// DEC r/m8 - Unified dispatch based on mod_c0()
pub fn dec_eb_dispatch<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        DEC_Eb(cpu, instr)
    } else {
        DEC_EbM(cpu, instr)
    }
}

// =========================================================================
// SUB - Accumulator optimized forms
// =========================================================================

/// SUB_AL_Ib: SUB AL, imm8
/// Optimized form for accumulator
/// Opcode: 0x2C
pub fn SUB_AL_Ib<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let al = cpu.al();
    let imm8 = instr.ib();
    let result = al.wrapping_sub(imm8);

    cpu.set_al(result);
    cpu.update_flags_sub8(al, imm8, result);

    // Trace vsprintf '%' detection: SUB AL, 0x25 in kernel space
    if imm8 == 0x25
        && cpu.rip() > 0xC0000000
        && cpu.icount > 100_000_000
        && cpu.icount < 200_000_000
    {
        let zf = cpu.get_zf();
        tracing::warn!(
            "SUB AL={:#04x}, 0x25 at RIP={:#x} result={:#04x} ZF={} icount={}",
            al,
            cpu.rip(),
            result,
            zf,
            cpu.icount
        );
    }

    Ok(())
}
