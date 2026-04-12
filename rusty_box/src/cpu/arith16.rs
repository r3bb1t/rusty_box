#![allow(non_snake_case, dead_code)]

// 16-bit arithmetic operations: ADD, ADC, SUB, SBB, CMP, INC, DEC
// Mirrors Bochs cpp/cpu/arith16.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // INC/DEC instructions
    // =========================================================================

    /// INC r16
    pub fn inc_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.wrapping_add(1);
        self.set_gpr16(dst, result);
        self.set_flags_oszap_inc_16(result, op1);
    }

    /// DEC r16
    pub fn dec_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.wrapping_sub(1);
        self.set_gpr16(dst, result);
        self.set_flags_oszap_dec_16(result, op1);
    }

    /// INC r/m16 (memory form) — matches Bochs INC_EwM
    pub fn inc_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_word(seg, eaddr)?;
        let result = op1.wrapping_add(1);
        self.write_rmw_linear_word(result);
        self.set_flags_oszap_inc_16(result, op1);
        Ok(())
    }

    /// DEC r/m16 (memory form) — matches Bochs DEC_EwM
    pub fn dec_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_word(seg, eaddr)?;
        let result = op1.wrapping_sub(1);
        self.write_rmw_linear_word(result);
        self.set_flags_oszap_dec_16(result, op1);
        Ok(())
    }

    /// INC r/m16 — unified dispatch
    pub fn inc_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.inc_ew_r(instr);
            Ok(())
        } else {
            self.inc_ew_m(instr)
        }
    }

    /// DEC r/m16 — unified dispatch
    pub fn dec_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.dec_ew_r(instr);
            Ok(())
        } else {
            self.dec_ew_m(instr)
        }
    }
}

// =========================================================================
// Free functions: 16-bit arithmetic (ADD, ADC, SUB, SBB, CMP)
// =========================================================================

/// ADC_GwEwR: ADC r16, r16 (register form)
/// Opcode: 0x13, ModRM: r16, r/m16 (register)
pub fn ADC_GwEwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADC_GwEwM: ADC r16, r/m16 (memory form)
/// Opcode: 0x13, ModRM: r16, r/m16 (memory)
pub fn ADC_GwEwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.v_read_word(seg, eaddr)?;
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADC_GwEw: ADC r16, r/m16
pub fn ADC_GwEw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_GwEwR(cpu, instr)
    } else {
        ADC_GwEwM(cpu, instr)
    }
}

/// ADD_EwIbR: ADD r/m16, imm8 (sign-extended, register form)
/// Opcode: 0x83/0
pub fn ADD_EwIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.ib() as i8 as i16 as u16;
    let result = op1.wrapping_add(op2);

    cpu.set_gpr16(dst, result);
    cpu.update_flags_add16(op1, op2, result);

    Ok(())
}

/// ADD_EwIbM: ADD r/m16, imm8 (sign-extended, memory form)
/// Opcode: 0x83/0 with memory operand
pub fn ADD_EwIbM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.ib() as i8 as i16 as u16;
    let result = op1.wrapping_add(op2);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_add16(op1, op2, result);
    Ok(())
}

/// ADD_EwIwR: ADD r16, imm16 (register form)
/// Opcode: 0x81/0
pub fn ADD_EwIwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1_16 = cpu.get_gpr16(dst);
    let op2_16 = instr.iw();
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(dst, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_EwIwM: ADD m16, imm16 (memory form)
/// Opcode: 0x81/0
pub fn ADD_EwIwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2_16 = instr.iw();
    let sum_16 = op1_16.wrapping_add(op2_16);
    cpu.write_rmw_linear_word(sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);
    Ok(())
}

/// ADD_EwsIb: ADD r/m16, imm8 (sign-extended) - combined dispatcher
/// Opcode: 0x83/0 with 66 prefix. Dispatches to register or memory form.
pub fn ADD_EwsIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_EwIbR(cpu, instr)
    } else {
        ADD_EwIbM(cpu, instr)
    }
}

/// ADD_EwIw: ADD r/m16, imm16
pub fn ADD_EwIw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_EwIwR(cpu, instr)
    } else {
        ADD_EwIwM(cpu, instr)
    }
}

/// ADD_EwGwM: ADD r/m16, r16 (memory form)
/// Opcode: 0x01
pub fn ADD_EwGwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let sum_16 = op1_16.wrapping_add(op2_16);
    cpu.write_rmw_linear_word(sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);
    Ok(())
}

/// ADD_EwGwR: ADD r/m16, r16 (register form)
/// Opcode 0x01: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn ADD_EwGwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_EwGw: ADD r/m16, r16
pub fn ADD_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_EwGwR(cpu, instr)
    } else {
        ADD_EwGwM(cpu, instr)
    }
}

/// ADC_EwGwM: ADC r/m16, r16 (memory form)
/// Opcode: 0x11
pub fn ADC_EwGwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);
    cpu.write_rmw_linear_word(sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);
    Ok(())
}

/// ADC_EwGwR: ADC r/m16, r16 (register form)
pub fn ADC_EwGwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let cf = cpu.get_cf() as u16;
    let sum_16 = op1_16.wrapping_add(op2_16).wrapping_add(cf);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADC_EwGw: ADC r/m16, r16
pub fn ADC_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EwGwR(cpu, instr)
    } else {
        ADC_EwGwM(cpu, instr)
    }
}

/// ADC_EwIbR: ADC r16, imm8 (sign-extended, register form)
/// Opcode: 0x83/2
pub fn ADC_EwIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.ib() as i8 as i16 as u16;
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_add(op2).wrapping_add(cf);

    cpu.set_gpr16(dst, result);
    cpu.update_flags_add16(op1, op2, result);

    Ok(())
}

/// ADC_EwIbM: ADC m16, imm8 (sign-extended, memory form)
/// Opcode: 0x83/2
pub fn ADC_EwIbM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.ib() as i8 as i16 as u16;
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_add16(op1, op2, result);
    Ok(())
}

/// ADC_EwsIb: ADC r/m16, imm8 (sign-extended) - dispatcher
pub fn ADC_EwsIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EwIbR(cpu, instr)
    } else {
        ADC_EwIbM(cpu, instr)
    }
}

// =========================================================================
// CMP - Compare (16-bit)
// =========================================================================

/// CMP_EwGwR: CMP r/m16, r16 (register form)
pub fn CMP_EwGwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let result = op1_16.wrapping_sub(op2_16);
    cpu.update_flags_sub16(op1_16, op2_16, result);
    Ok(())
}

/// CMP_EwGwM: CMP r/m16, r16 (memory form)
pub fn CMP_EwGwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_word(seg, eaddr)?;
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let result = op1_16.wrapping_sub(op2_16);
    cpu.update_flags_sub16(op1_16, op2_16, result);
    Ok(())
}

/// CMP_EwGw: CMP r/m16, r16 - Dispatcher
pub fn CMP_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_EwGwR(cpu, instr)
    } else {
        CMP_EwGwM(cpu, instr)
    }
}

// =========================================================================
// ADD r16, r/m16 (GwEw direction)
// =========================================================================

/// ADD_GwEwR: ADD r16, r16 (register form)
pub fn ADD_GwEwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_GwEwM: ADD r16, r/m16 (memory form)
pub fn ADD_GwEwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.v_read_word(seg, eaddr)?;
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(instr.dst() as usize, sum_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);

    Ok(())
}

/// ADD_GwEw: ADD r16, r/m16 - unified dispatch
pub fn ADD_GwEw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_GwEwR(cpu, instr)
    } else {
        ADD_GwEwM(cpu, instr)
    }
}

// =========================================================================
// SUB r/m16, r16 (EwGw direction) and SUB r16, r/m16 (GwEw direction)
// =========================================================================

/// SUB_EwGwM: SUB r/m16, r16 (memory form)
pub fn SUB_EwGwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let diff_16 = op1_16.wrapping_sub(op2_16);
    cpu.write_rmw_linear_word(diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);
    Ok(())
}

/// SUB_EwGwR: SUB r/m16, r16 (register form)
pub fn SUB_EwGwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let diff_16 = op1_16.wrapping_sub(op2_16);

    cpu.set_gpr16(instr.dst() as usize, diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);

    Ok(())
}

/// SUB_EwGw: SUB r/m16, r16 - unified dispatch
pub fn SUB_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_EwGwR(cpu, instr)
    } else {
        SUB_EwGwM(cpu, instr)
    }
}

/// SUB_GwEwR: SUB r16, r16 (register form)
pub fn SUB_GwEwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let diff_16 = op1_16.wrapping_sub(op2_16);

    cpu.set_gpr16(instr.dst() as usize, diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);

    Ok(())
}

/// SUB_GwEwM: SUB r16, r/m16 (memory form)
pub fn SUB_GwEwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.v_read_word(seg, eaddr)?;
    let diff_16 = op1_16.wrapping_sub(op2_16);

    cpu.set_gpr16(instr.dst() as usize, diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);

    Ok(())
}

/// SUB_GwEw: SUB r16, r/m16 - unified dispatch
pub fn SUB_GwEw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_GwEwR(cpu, instr)
    } else {
        SUB_GwEwM(cpu, instr)
    }
}

// =========================================================================
// SBB r/m16, r16 (EwGw direction) and SBB r16, r/m16 (GwEw direction)
// =========================================================================

/// SBB_EwGwM: SBB r/m16, r16 (memory form)
pub fn SBB_EwGwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let cf = cpu.get_cf() as u16;
    let diff_16 = op1_16.wrapping_sub(op2_16).wrapping_sub(cf);
    cpu.write_rmw_linear_word(diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);
    Ok(())
}

/// SBB_EwGwR: SBB r/m16, r16 (register form)
pub fn SBB_EwGwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let cf = cpu.get_cf() as u16;
    let diff_16 = op1_16.wrapping_sub(op2_16).wrapping_sub(cf);

    cpu.set_gpr16(instr.dst() as usize, diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);

    Ok(())
}

/// SBB_EwGw: SBB r/m16, r16 - unified dispatch
pub fn SBB_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EwGwR(cpu, instr)
    } else {
        SBB_EwGwM(cpu, instr)
    }
}

/// SBB_GwEwR: SBB r16, r16 (register form)
pub fn SBB_GwEwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let cf = cpu.get_cf() as u16;
    let diff_16 = op1_16.wrapping_sub(op2_16).wrapping_sub(cf);

    cpu.set_gpr16(instr.dst() as usize, diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);

    Ok(())
}

/// SBB_GwEwM: SBB r16, r/m16 (memory form)
pub fn SBB_GwEwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.v_read_word(seg, eaddr)?;
    let cf = cpu.get_cf() as u16;
    let diff_16 = op1_16.wrapping_sub(op2_16).wrapping_sub(cf);

    cpu.set_gpr16(instr.dst() as usize, diff_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);

    Ok(())
}

/// SBB_GwEw: SBB r16, r/m16 - unified dispatch
pub fn SBB_GwEw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_GwEwR(cpu, instr)
    } else {
        SBB_GwEwM(cpu, instr)
    }
}

// =========================================================================
// CMP r16, r/m16 (GwEw direction)
// =========================================================================

/// CMP_GwEwR: CMP r16, r16 (register form)
pub fn CMP_GwEwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src1() as usize);
    let diff_16 = op1_16.wrapping_sub(op2_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);
    Ok(())
}

/// CMP_GwEwM: CMP r16, r/m16 (memory form)
pub fn CMP_GwEwM<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.v_read_word(seg, eaddr)?;
    let diff_16 = op1_16.wrapping_sub(op2_16);
    cpu.update_flags_sub16(op1_16, op2_16, diff_16);
    Ok(())
}

/// CMP_GwEw: CMP r16, r/m16 - unified dispatch
pub fn CMP_GwEw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_GwEwR(cpu, instr)
    } else {
        CMP_GwEwM(cpu, instr)
    }
}

// =========================================================================
// ADD - Accumulator optimized forms
// =========================================================================

/// ADD_Axiw: ADD AX, imm16
pub fn ADD_Axiw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let ax = cpu.ax();
    let imm16 = instr.iw();
    let result = ax.wrapping_add(imm16);

    cpu.set_ax(result);
    cpu.update_flags_add16(ax, imm16, result);

    Ok(())
}

/// SUB_AX_Iw: SUB AX, imm16
pub fn SUB_AX_Iw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let ax = cpu.ax();
    let imm16 = instr.iw();
    let result = ax.wrapping_sub(imm16);

    cpu.set_ax(result);
    cpu.update_flags_sub16(ax, imm16, result);

    Ok(())
}

/// SUB_EwIwR: SUB r16, imm16 (register form)
pub fn SUB_EwIwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.iw();
    let result = op1.wrapping_sub(op2);

    cpu.set_gpr16(dst, result);
    cpu.update_flags_sub16(op1, op2, result);

    Ok(())
}

/// SUB_EwIwM: SUB m16, imm16 (memory form)
pub fn SUB_EwIwM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.iw();
    let result = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// SUB_EwIw: SUB r/m16, imm16 - dispatcher
pub fn SUB_EwIw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_EwIwR(cpu, instr)
    } else {
        SUB_EwIwM(cpu, instr)
    }
}

/// SUB_EwIbR: SUB r16, imm8 (sign-extended, register form)
pub fn SUB_EwIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.ib() as i8 as i16 as u16;
    let result = op1.wrapping_sub(op2);

    cpu.set_gpr16(dst, result);
    cpu.update_flags_sub16(op1, op2, result);

    Ok(())
}

/// SUB_EwIbM: SUB m16, imm8 (sign-extended, memory form)
pub fn SUB_EwIbM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.ib() as i8 as i16 as u16;
    let result = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// SUB_EwsIb: SUB r/m16, imm8 (sign-extended) - dispatcher
pub fn SUB_EwsIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_EwIbR(cpu, instr)
    } else {
        SUB_EwIbM(cpu, instr)
    }
}

/// ADC_AX_Iw: ADC AX, imm16 (opcode 0x15) - Bochs ADC_AXIw
pub fn ADC_AX_Iw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let ax = cpu.ax();
    let imm16 = instr.iw();
    let cf = cpu.get_cf() as u16;
    let result = ax.wrapping_add(imm16).wrapping_add(cf);
    cpu.set_ax(result);
    cpu.update_flags_add16(ax, imm16, result);
    Ok(())
}

/// ADC_EwIwR: ADC r16, imm16 (register form, opcode 0x81 /2)
pub fn ADC_EwIwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.iw();
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.set_gpr16(dst, result);
    cpu.update_flags_add16(op1, op2, result);
    Ok(())
}

/// ADC_EwIwM: ADC m16, imm16 (memory form, opcode 0x81 /2)
pub fn ADC_EwIwM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.iw();
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_add16(op1, op2, result);
    Ok(())
}

/// ADC_EwIw: ADC r/m16, imm16 - dispatcher (Bochs AdcEwIw)
pub fn ADC_EwIw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EwIwR(cpu, instr)
    } else {
        ADC_EwIwM(cpu, instr)
    }
}

/// SBB_AX_Iw: SBB AX, imm16 (opcode 0x1D) - Bochs SBB_AXIw
pub fn SBB_AX_Iw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let ax = cpu.ax();
    let imm16 = instr.iw();
    let cf = cpu.get_cf() as u16;
    let result = ax.wrapping_sub(imm16).wrapping_sub(cf);
    cpu.set_ax(result);
    cpu.update_flags_sub16(ax, imm16, result);
    Ok(())
}

/// SBB_EwIwR: SBB r16, imm16 (register form, opcode 0x81 /3)
pub fn SBB_EwIwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.iw();
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr16(dst, result);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// SBB_EwIwM: SBB m16, imm16 (memory form, opcode 0x81 /3)
pub fn SBB_EwIwM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.iw();
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// SBB_EwIw: SBB r/m16, imm16 - dispatcher (Bochs SbbEwIw)
pub fn SBB_EwIw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EwIwR(cpu, instr)
    } else {
        SBB_EwIwM(cpu, instr)
    }
}

/// SBB_EwIbR: SBB r16, imm8 sign-extended (register form, opcode 0x83 /3)
pub fn SBB_EwIbR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.ib() as i8 as i16 as u16;
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr16(dst, result);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// SBB_EwIbM: SBB m16, imm8 sign-extended (memory form, opcode 0x83 /3)
pub fn SBB_EwIbM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2 = instr.ib() as i8 as i16 as u16;
    let cf = cpu.get_cf() as u16;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_word(result);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// SBB_EwsIb: SBB r/m16, imm8 sign-extended - dispatcher (Bochs SbbEwsIb)
pub fn SBB_EwsIb<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EwIbR(cpu, instr)
    } else {
        SBB_EwIbM(cpu, instr)
    }
}

/// CMP_EwIwR: CMP r16, imm16 (register form)
pub fn CMP_EwIwR<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr16(dst);
    let op2 = instr.iw();
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// CMP_EwIwM: CMP m16, imm16 (memory form)
pub fn CMP_EwIwM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_word(seg, eaddr)?;
    let op2 = instr.iw();
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub16(op1, op2, result);
    Ok(())
}

/// CMP_EwIw: CMP r/m16, imm16 - dispatcher
pub fn CMP_EwIw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_EwIwR(cpu, instr)
    } else {
        CMP_EwIwM(cpu, instr)
    }
}

// =========================================================================
// CMPXCHG — Compare and Exchange (opcode 0x0F B1, operand-size 16)
// Matches Bochs arith16.cc CMPXCHG_EwGwR / CMPXCHG_EwGwM
// =========================================================================

/// CMPXCHG r/m16, r16 — register form
/// Bochs  (CMPXCHG_EwGwR)
pub fn CMPXCHG_EwGw_R<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &Instruction) {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let ax = cpu.ax();
    let diff_16 = ax.wrapping_sub(op1_16);
    cpu.update_flags_sub16(ax, op1_16, diff_16);

    if diff_16 == 0 {
        cpu.set_gpr16(instr.dst() as usize, cpu.get_gpr16(instr.src() as usize));
    } else {
        cpu.set_ax(op1_16);
    }
}

/// CMPXCHG r/m16, r16 — memory form
/// Bochs  (CMPXCHG_EwGwM)
pub fn CMPXCHG_EwGw_M<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let ax = cpu.ax();
    let diff_16 = ax.wrapping_sub(op1_16);
    cpu.update_flags_sub16(ax, op1_16, diff_16);

    if diff_16 == 0 {
        cpu.write_rmw_linear_word(cpu.get_gpr16(instr.src() as usize));
    } else {
        cpu.write_rmw_linear_word(op1_16);
        cpu.set_ax(op1_16);
    }
    Ok(())
}

// =========================================================================
// XADD — Exchange and Add (opcode 0x0F C1, operand-size 16)
// Matches Bochs arith16.cc XADD_EwGwR / XADD_EwGwM
// =========================================================================

/// XADD r/m16, r16 — register form
/// Bochs 
pub fn XADD_EwGw_R<'c, I: BxCpuIdTrait>(cpu: &mut BxCpuC<'c, I>, instr: &Instruction) {
    let op1_16 = cpu.get_gpr16(instr.dst() as usize);
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.set_gpr16(instr.src() as usize, op1_16);
    cpu.set_gpr16(instr.dst() as usize, sum_16);

    cpu.update_flags_add16(op1_16, op2_16, sum_16);
}

/// XADD r/m16, r16 — memory form
/// Bochs 
pub fn XADD_EwGw_M<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_16 = cpu.v_read_rmw_word(seg, eaddr)?;
    let op2_16 = cpu.get_gpr16(instr.src() as usize);
    let sum_16 = op1_16.wrapping_add(op2_16);

    cpu.write_rmw_linear_word(sum_16);
    cpu.set_gpr16(instr.src() as usize, op1_16);
    cpu.update_flags_add16(op1_16, op2_16, sum_16);
    Ok(())
}

// =========================================================================
// CMPXCHG - unified dispatch (16-bit)
// =========================================================================

/// CMPXCHG r/m16, r16 — unified dispatch
pub fn CMPXCHG_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMPXCHG_EwGw_R(cpu, instr);
        Ok(())
    } else {
        CMPXCHG_EwGw_M(cpu, instr)
    }
}

// =========================================================================
// XADD - unified dispatch (16-bit)
// =========================================================================

/// XADD r/m16, r16 — unified dispatch
pub fn XADD_EwGw<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        XADD_EwGw_R(cpu, instr);
        Ok(())
    } else {
        XADD_EwGw_M(cpu, instr)
    }
}

// =========================================================================
// NEG - Two's Complement Negation (16-bit)
// Matching Bochs arith16.cc NEG_EwR / NEG_EwM
// =========================================================================

/// NEG r/m16 - unified dispatch
pub fn NEG_Ew<'c, I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'c, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        let dst = instr.dst() as usize;
        let op1 = cpu.get_gpr16(dst);
        let result = 0u16.wrapping_sub(op1);
        cpu.set_gpr16(dst, result);
        cpu.update_flags_sub16(0, op1, result);
        Ok(())
    } else {
        let eaddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = cpu.v_read_rmw_word(seg, eaddr)?;
        let result = 0u16.wrapping_sub(op1);
        cpu.write_rmw_linear_word(result);
        cpu.update_flags_sub16(0, op1, result);
        Ok(())
    }
}
