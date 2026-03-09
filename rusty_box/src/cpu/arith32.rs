// 32-bit arithmetic operations: ADD, SUB, etc.
// Mirrors Bochs cpp/cpu/arith32.cc

use crate::cpu::decoder::{BxSegregs, Instruction};
use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// ADD_GdEd_R: ADD r32, r/m32 (register form)
/// Opcode: 0x03, ModRM: r32, r/m32 (register)
/// operands.dst = destination register
/// operands.src1 = source register
pub fn ADD_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_idx = instr.operands.dst as usize;
    let src_idx = instr.operands.src1 as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_add(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_add32(dst_val, src_val, result);
}

/// ADD_EdGd_R: ADD r/m32, r32 (register form)
/// Opcode: 0x01: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn ADD_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize); // rm = destination/first operand
    let op2 = cpu.get_gpr32(instr.operands.src1 as usize); // nnn = source/second operand
    let result = op1.wrapping_add(op2);

    cpu.set_gpr32(instr.operands.dst as usize, result); // write to rm = destination
    cpu.update_flags_add32(op1, op2, result);
}

/// ADD_EAX_Id: ADD EAX, imm32
/// Opcode: 0x05
/// Immediate value stored in operand_data.Id
pub fn ADD_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let eax = cpu.eax();
    let imm: u32 = instr.immediate;
    let result = eax.wrapping_add(imm);

    cpu.set_eax(result);
    cpu.update_flags_add32(eax, imm, result);
}

/// ADD_EdId_R: ADD r32, imm32 (register form, sign-extended immediate)
/// Original: bochs/cpu/arith32.cc ADD_EdIdR
/// Opcode: 0x81/0x83, ModRM: r/m32, imm32/imm8
pub fn ADD_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_reg = instr.operands.dst as usize;
    let op1 = cpu.get_gpr32(dst_reg);
    let op2 = instr.id(); // Sign-extended immediate
    let result = op1.wrapping_add(op2);

    cpu.set_gpr32(dst_reg, result);
    cpu.update_flags_add32(op1, op2, result);
}

/// SUB_GdEd_R: SUB r32, r/m32 (register form)
/// Opcode: 0x2B, ModRM: r32, r/m32 (register)
pub fn SUB_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_idx = instr.operands.dst as usize;
    let src_idx = instr.operands.src1 as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_sub(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_sub32(dst_val, src_val, result);
}

/// SUB_EdGd_R: SUB r/m32, r32 (register form)
/// Opcode: 0x29: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn SUB_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize); // rm = destination/first operand
    let op2 = cpu.get_gpr32(instr.operands.src1 as usize); // nnn = source/second operand
    let result = op1.wrapping_sub(op2);

    cpu.set_gpr32(instr.operands.dst as usize, result); // write to rm = destination
    cpu.update_flags_sub32(op1, op2, result);
}

/// SUB_EAX_Id: SUB EAX, imm32
/// Opcode: 0x2D
pub fn SUB_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let eax = cpu.eax();
    let imm: u32 = instr.immediate;
    let result = eax.wrapping_sub(imm);

    cpu.set_eax(result);
    cpu.update_flags_sub32(eax, imm, result);
}

/// SUB_EdId_R: SUB r32, imm32 (register form, sign-extended immediate)
/// Original: bochs/cpu/arith32.cc SUB_EdIdR
/// Opcode: 0x81/0x83, ModRM: r/m32, imm32/imm8
pub fn SUB_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_reg = instr.operands.dst as usize;
    let op1 = cpu.get_gpr32(dst_reg);
    let op2 = instr.id(); // Sign-extended immediate
    let result = op1.wrapping_sub(op2);

    cpu.set_gpr32(dst_reg, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// CMP_EdId_R: CMP r32, imm32 (register form, sign-extended immediate)
/// Original: bochs/cpu/arith32.cc CMP_EdIdR
/// Opcode: 0x81/0x83, ModRM: r/m32, imm32/imm8
pub fn CMP_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_reg = instr.operands.dst as usize;
    let op1 = cpu.get_gpr32(dst_reg);
    let op2 = instr.id(); // Sign-extended immediate
    let result = op1.wrapping_sub(op2);

    // CMP only sets flags, doesn't write result
    cpu.update_flags_sub32(op1, op2, result);
    // Trace '%' (0x25=37) comparisons in kernel space
    if op2 == 0x25 && op1 == 0x25 && cpu.rip() > 0xC0000000 {
        let zf = (cpu.eflags.bits() >> 6) & 1;
        tracing::warn!(
            "CMP Ed=0x25, Id=0x25 at RIP={:#x} ZF={} eflags={:#x} icount={} reg={}",
            cpu.rip(),
            zf,
            cpu.eflags.bits(),
            cpu.icount,
            dst_reg
        );
    }
}

/// CMP_EdGd_R: CMP r/m32, r32 (register form)
/// Opcode: 0x39: decoder swaps for 16/32-bit store: [0]=rm=first operand, [1]=nnn=second operand
/// Performs rm - nnn and sets flags without storing result
fn CMP_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize); // rm = first operand
    let op2 = cpu.get_gpr32(instr.operands.src1 as usize); // nnn = second operand
    let diff = op1.wrapping_sub(op2);

    cpu.update_flags_sub32(op1, op2, diff);

    tracing::trace!(
        "CMP r{}d, r{}d: {:#010x} - {:#010x}",
        instr.operands.dst,
        instr.operands.src1,
        op1,
        op2
    );
}

/// ADC_EdGd_R: ADC r/m32, r32 (register form)
/// Opcode: 0x11: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn ADC_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize); // rm = destination/first operand
    let op2 = cpu.get_gpr32(instr.operands.src1 as usize); // nnn = source/second operand
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);

    cpu.set_gpr32(instr.operands.dst as usize, result); // write to rm = destination
    cpu.update_flags_add32(op1, op2, result);
}

/// ADC_GdEd_R: ADC r32, r/m32 (register form)
/// Original: bochs/cpu/arith32.cc ADC_GdEd (register case)
/// Opcode: 0x13, ModRM: r32, r/m32 (register)
pub fn ADC_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst_idx = instr.operands.dst as usize;
    let src_idx = instr.operands.src1 as usize;

    let op1_32 = cpu.get_gpr32(dst_idx);
    let op2_32 = cpu.get_gpr32(src_idx);
    let cf = cpu.get_cf() as u32;
    let sum_32 = op1_32.wrapping_add(op2_32).wrapping_add(cf);

    cpu.set_gpr32(dst_idx, sum_32);
    cpu.update_flags_add32(op1_32, op2_32, sum_32);
}

// =========================================================================
// Memory-form handlers (mod != 11)
// Mirrors Bochs arith32.cc *M functions
// =========================================================================

/// ADD_EdGd_M: ADD r/m32, r32 (memory form) - read-modify-write
/// Decoder swaps: src() = operands.src1 = nnn = SOURCE register
pub fn ADD_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let result = op1.wrapping_add(op2);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADD_GdEd_M: ADD r32, r/m32 (memory form) - read memory, write register
pub fn ADD_GdEd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.v_read_dword(seg, eaddr)?;
    let result = op1.wrapping_add(op2);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADD_EdId_M: ADD r/m32, imm32 (memory form)
pub fn ADD_EdId_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = instr.id();
    let result = op1.wrapping_add(op2);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// SUB_EdGd_M: SUB r/m32, r32 (memory form)
/// Decoder swaps: src() = operands.src1 = nnn = SOURCE register
pub fn SUB_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let result = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SUB_GdEd_M: SUB r32, r/m32 (memory form)
pub fn SUB_GdEd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.v_read_dword(seg, eaddr)?;
    let result = op1.wrapping_sub(op2);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SUB_EdId_M: SUB r/m32, imm32 (memory form)
pub fn SUB_EdId_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = instr.id();
    let result = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// CMP_EdGd_M: CMP r/m32, r32 (memory form)
/// Decoder swaps: src() = operands.src1 = nnn = SOURCE register
pub fn CMP_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// CMP_GdEd_M: CMP r32, r/m32 (memory form)
pub fn CMP_GdEd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.v_read_dword(seg, eaddr)?;
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// CMP_EdId_M: CMP r/m32, imm32 (memory form)
pub fn CMP_EdId_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_dword(seg, eaddr)?;
    let op2 = instr.id();
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// ADC_EdGd_M: ADC r/m32, r32 (memory form)
/// Decoder swaps: src() = operands.src1 = nnn = SOURCE register
pub fn ADC_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADC_GdEd_M: ADC r32, r/m32 (memory form)
pub fn ADC_GdEd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.v_read_dword(seg, eaddr)?;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

// =========================================================================
// Unified handlers: dispatch R/M based on instr.mod_c0()
//
// Each unified handler checks mod_c0() internally, mirroring Bochs'
// execute() pattern where the decoder pre-assigns R or M handlers.
// Here we resolve at execution time with a single branch, keeping the
// decoder crate independent of the CPU implementation.
// =========================================================================

/// ADD r/m32, r32 - unified (Bochs: ADD_EdGdR / ADD_EdGdM)
pub fn ADD_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_EdGd_R(cpu, instr);
        Ok(())
    } else {
        ADD_EdGd_M(cpu, instr)
    }
}

/// ADD r32, r/m32 - unified (Bochs: ADD_GdEdR / ADD_GdEdM)
pub fn ADD_GdEd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_GdEd_R(cpu, instr);
        Ok(())
    } else {
        ADD_GdEd_M(cpu, instr)
    }
}

/// ADD r/m32, imm32 - unified (Bochs: ADD_EdIdR / ADD_EdIdM)
pub fn ADD_EdId<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADD_EdId_R(cpu, instr);
        Ok(())
    } else {
        ADD_EdId_M(cpu, instr)
    }
}

/// SUB r/m32, r32 - unified (Bochs: SUB_EdGdR / SUB_EdGdM)
pub fn SUB_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_EdGd_R(cpu, instr);
        Ok(())
    } else {
        SUB_EdGd_M(cpu, instr)
    }
}

/// SUB r32, r/m32 - unified (Bochs: SUB_GdEdR / SUB_GdEdM)
pub fn SUB_GdEd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_GdEd_R(cpu, instr);
        Ok(())
    } else {
        SUB_GdEd_M(cpu, instr)
    }
}

/// SUB r/m32, imm32 - unified (Bochs: SUB_EdIdR / SUB_EdIdM)
pub fn SUB_EdId<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SUB_EdId_R(cpu, instr);
        Ok(())
    } else {
        SUB_EdId_M(cpu, instr)
    }
}

/// CMP r/m32, r32 - unified (Bochs: CMP_EdGdR / CMP_EdGdM)
pub fn CMP_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_EdGd_R(cpu, instr);
        Ok(())
    } else {
        CMP_EdGd_M(cpu, instr)
    }
}

/// CMP r32, r/m32 - unified (Bochs: CMP_GdEdR / CMP_GdEdM)
pub fn CMP_GdEd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_EdGd_R(cpu, instr);
        Ok(())
    } else {
        CMP_GdEd_M(cpu, instr)
    }
}

/// CMP r/m32, imm32 - unified (Bochs: CMP_EdIdR / CMP_EdIdM)
pub fn CMP_EdId<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMP_EdId_R(cpu, instr);
        Ok(())
    } else {
        CMP_EdId_M(cpu, instr)
    }
}

/// ADC r/m32, r32 - unified (Bochs: ADC_EdGdR / ADC_EdGdM)
pub fn ADC_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EdGd_R(cpu, instr);
        Ok(())
    } else {
        ADC_EdGd_M(cpu, instr)
    }
}

/// ADC r32, r/m32 - unified (Bochs: ADC_GdEdR / ADC_GdEdM)
pub fn ADC_GdEd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_GdEd_R(cpu, instr);
        Ok(())
    } else {
        ADC_GdEd_M(cpu, instr)
    }
}

/// ADC EAX, imm32 (opcode 0x15) - Bochs ADC_EAXId
pub fn ADC_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.eax();
    let op2 = instr.id();
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.set_eax(result);
    cpu.update_flags_add32(op1, op2, result);
}

/// ADC r/m32, imm32 - register form (opcode 0x81 /2) - Bochs ADC_EdIdR
pub fn ADC_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize);
    let op2 = instr.id();
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.set_gpr32(instr.operands.dst as usize, result);
    cpu.update_flags_add32(op1, op2, result);
}

/// ADC r/m32, imm32 - memory form (opcode 0x81 /2) - Bochs ADC_EdIdM
pub fn ADC_EdId_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = instr.id();
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADC r/m32, imm32 - unified (Bochs ADC_EdIdR / ADC_EdIdM)
pub fn ADC_EdId<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EdId_R(cpu, instr);
        Ok(())
    } else {
        ADC_EdId_M(cpu, instr)
    }
}

/// ADC r/m32, imm8 sign-extended - register form (opcode 0x83 /2) - Bochs ADC_EdIbR
pub fn ADC_EdIb_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize);
    let op2 = instr.ib() as i8 as i32 as u32;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.set_gpr32(instr.operands.dst as usize, result);
    cpu.update_flags_add32(op1, op2, result);
}

/// ADC r/m32, imm8 sign-extended - memory form (opcode 0x83 /2) - Bochs ADC_EdIbM
pub fn ADC_EdIb_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = instr.ib() as i8 as i32 as u32;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADC r/m32, imm8 sign-extended - unified (Bochs ADC_EdsIb)
pub fn ADC_EdsIb<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        ADC_EdIb_R(cpu, instr);
        Ok(())
    } else {
        ADC_EdIb_M(cpu, instr)
    }
}

/// SBB r/m32, imm32 - memory form (opcode 0x81 /3) - Bochs SBB_EdIdM
pub fn SBB_EdId_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = instr.id();
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SBB r/m32, imm32 - unified (Bochs SBB_EdIdR / SBB_EdIdM)
pub fn SBB_EdId<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EdId_R(cpu, instr);
        Ok(())
    } else {
        SBB_EdId_M(cpu, instr)
    }
}

/// SBB r/m32, imm8 sign-extended - memory form (opcode 0x83 /3) - Bochs SBB_EdIbM
pub fn SBB_EdIb_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = instr.ib() as i8 as i32 as u32;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SBB r/m32, imm8 sign-extended - unified (Bochs SBB_EdsIb)
pub fn SBB_EdsIb<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EdIb_R(cpu, instr);
        Ok(())
    } else {
        SBB_EdIb_M(cpu, instr)
    }
}

// =========================================================================
// NEG - Two's Complement Negation (32-bit)
// Matching Bochs arith32.cc NEG_EdR / NEG_EdM
// =========================================================================

/// NEG r32 - Negate register (register form)
pub fn NEG_EdR<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr32(dst);
    let result = 0u32.wrapping_sub(op1);
    cpu.set_gpr32(dst, result);
    cpu.update_flags_sub32(0, op1, result);
}

/// NEG m32 - Negate memory (memory form)
pub fn NEG_EdM<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let result = 0u32.wrapping_sub(op1);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_sub32(0, op1, result);
    Ok(())
}

/// NEG r/m32 - unified dispatch
pub fn NEG_Ed<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        NEG_EdR(cpu, instr);
        Ok(())
    } else {
        NEG_EdM(cpu, instr)
    }
}

// =========================================================================
// SBB (Subtract with Borrow) - 32-bit
// =========================================================================

/// SBB r/m32, r32 - register form (opcode 0x19)
pub fn SBB_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize);
    let op2 = cpu.get_gpr32(instr.operands.src1 as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.operands.dst as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// SBB r/m32, r32 - memory form
pub fn SBB_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_dword(result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SBB r/m32, r32 - unified dispatch
pub fn SBB_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_EdGd_R(cpu, instr);
        Ok(())
    } else {
        SBB_EdGd_M(cpu, instr)
    }
}

/// SBB r32, r/m32 - register form (opcode 0x1B)
pub fn SBB_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize);
    let op2 = cpu.get_gpr32(instr.operands.src1 as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.operands.dst as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// SBB r32, r/m32 - memory form
pub fn SBB_GdEd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.v_read_dword(seg, eaddr)?;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SBB r32, r/m32 - unified dispatch
pub fn SBB_GdEd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        SBB_GdEd_R(cpu, instr);
        Ok(())
    } else {
        SBB_GdEd_M(cpu, instr)
    }
}

/// SBB EAX, imm32 (opcode 0x1D)
pub fn SBB_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let eax = cpu.eax();
    let imm = instr.immediate;
    let cf = cpu.get_cf() as u32;
    let result = eax.wrapping_sub(imm).wrapping_sub(cf);
    cpu.set_eax(result);
    cpu.update_flags_sub32(eax, imm, result);
}

/// SBB r/m32, imm32 (opcode 0x81 /3) - register form
pub fn SBB_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize);
    let op2 = instr.immediate;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.operands.dst as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// SBB r/m32, imm8 sign-extended (opcode 0x83 /3) - register form
pub fn SBB_EdIb_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1 = cpu.get_gpr32(instr.operands.dst as usize);
    let op2 = instr.ib() as i8 as i32 as u32;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.operands.dst as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

// =========================================================================
// CMPXCHG — Compare and Exchange (opcode 0x0F B1)
// Matches Bochs arith32.cc CMPXCHG_EdGdR / CMPXCHG_EdGdM
// =========================================================================

/// CMPXCHG r/m32, r32 — register form
/// Bochs arith32.cc:561-577
/// Compare EAX with destination; if equal, load source into dest.
/// Otherwise, load dest into EAX. Flags set from the comparison.
/// CMPXCHG r/m32, r32 — register form
/// Bochs arith32.cc:561-577 (CMPXCHG_EdGdR)
pub fn CMPXCHG_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1_32 = cpu.get_gpr32(instr.dst() as usize);
    let diff_32 = cpu.eax().wrapping_sub(op1_32);
    cpu.update_flags_sub32(cpu.eax(), op1_32, diff_32);

    if diff_32 == 0 {
        cpu.set_gpr32(instr.dst() as usize, cpu.get_gpr32(instr.src() as usize));
    } else {
        cpu.set_rax(op1_32 as u64);
    }
}

/// CMPXCHG r/m32, r32 — memory form
/// Bochs arith32.cc:540-558 (CMPXCHG_EdGdM)
pub fn CMPXCHG_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_32 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let diff_32 = cpu.eax().wrapping_sub(op1_32);
    cpu.update_flags_sub32(cpu.eax(), op1_32, diff_32);

    if diff_32 == 0 {
        // dest <- src
        cpu.write_rmw_linear_dword(cpu.get_gpr32(instr.src() as usize));
    } else {
        // write back original value (Bochs: write_RMW_linear_dword(op1_32))
        cpu.write_rmw_linear_dword(op1_32);
        cpu.set_rax(op1_32 as u64);
    }
    Ok(())
}

/// CMPXCHG8B m64 — Compare and Exchange 8 Bytes
/// Bochs arith32.cc:579-602 (CMPXCHG8B)
/// Compares EDX:EAX with m64. If equal, sets ZF and stores ECX:EBX into m64.
/// Otherwise, clears ZF and loads m64 into EDX:EAX.
pub fn CMPXCHG8B<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());

    // Read the qword as two dwords (RMW access)
    let lo = cpu.v_read_rmw_dword(seg, eaddr)?;
    // Save address_xlation state for first dword
    let xlat_saved = cpu.address_xlation;
    let hi = cpu.v_read_dword(seg, eaddr.wrapping_add(4))?;

    let op1_64 = ((hi as u64) << 32) | (lo as u64);
    let op2_64 = ((cpu.edx() as u64) << 32) | (cpu.eax() as u64);

    if op1_64 == op2_64 {
        // dest <- ECX:EBX
        let src_64 = ((cpu.ecx() as u64) << 32) | (cpu.ebx() as u64);
        // Restore xlation for the first dword write-back
        cpu.address_xlation = xlat_saved;
        cpu.write_rmw_linear_dword(src_64 as u32);
        cpu.v_write_dword(seg, eaddr.wrapping_add(4), (src_64 >> 32) as u32)?;
        cpu.eflags.insert(crate::cpu::eflags::EFlags::ZF);
    } else {
        // EDX:EAX <- dest
        // Write back original value (Bochs: write_RMW_linear_qword(op1_64))
        cpu.address_xlation = xlat_saved;
        cpu.write_rmw_linear_dword(lo);
        cpu.set_rax(lo as u64);
        cpu.set_rdx(hi as u64);
        cpu.eflags.remove(crate::cpu::eflags::EFlags::ZF);
    }
    Ok(())
}

// =========================================================================
// XADD — Exchange and Add (opcode 0x0F C1, operand-size 32)
// Matches Bochs arith32.cc XADD_EdGdR / XADD_EdGdM
// =========================================================================

/// XADD r/m32, r32 — register form
/// Bochs arith32.cc:349-373
pub fn XADD_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &Instruction) {
    let op1_32 = cpu.get_gpr32(instr.dst() as usize);
    let op2_32 = cpu.get_gpr32(instr.src() as usize);
    let sum_32 = op1_32.wrapping_add(op2_32);

    // Note: write source BEFORE destination, so if src==dst the sum wins
    cpu.set_gpr32(instr.src() as usize, op1_32);
    cpu.set_gpr32(instr.dst() as usize, sum_32);

    cpu.update_flags_add32(op1_32, op2_32, sum_32);
}

// =========================================================================
// CMPXCHG - unified dispatch (32-bit)
// =========================================================================

/// CMPXCHG r/m32, r32 — unified dispatch
pub fn CMPXCHG_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        CMPXCHG_EdGd_R(cpu, instr);
        Ok(())
    } else {
        CMPXCHG_EdGd_M(cpu, instr)
    }
}

// =========================================================================
// XADD - unified dispatch (32-bit)
// =========================================================================

/// XADD r/m32, r32 — unified dispatch
pub fn XADD_EdGd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() {
        XADD_EdGd_R(cpu, instr);
        Ok(())
    } else {
        XADD_EdGd_M(cpu, instr)
    }
}

/// XADD r/m32, r32 — memory form
/// Bochs arith32.cc:324-347
pub fn XADD_EdGd_M<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1_32 = cpu.v_read_rmw_dword(seg, eaddr)?;
    let op2_32 = cpu.get_gpr32(instr.src() as usize);
    let sum_32 = op1_32.wrapping_add(op2_32);

    cpu.write_rmw_linear_dword(sum_32);
    cpu.set_gpr32(instr.src() as usize, op1_32);
    cpu.update_flags_add32(op1_32, op2_32, sum_32);
    Ok(())
}
