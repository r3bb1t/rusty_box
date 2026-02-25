// 32-bit arithmetic operations: ADD, SUB, etc.
// Mirrors Bochs cpp/cpu/arith32.cc

use crate::cpu::decoder::{BxInstructionGenerated, BxSegregs};
use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// ADD_GdEd_R: ADD r32, r/m32 (register form)
/// Opcode: 0x03, ModRM: r32, r/m32 (register)
/// meta_data[0] = destination register
/// meta_data[1] = source register
pub fn ADD_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_add(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_add32(dst_val, src_val, result);
}

/// ADD_EdGd_R: ADD r/m32, r32 (register form)
/// Opcode: 0x01: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn ADD_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);  // rm = destination/first operand
    let op2 = cpu.get_gpr32(instr.meta_data[1] as usize);  // nnn = source/second operand
    let result = op1.wrapping_add(op2);

    cpu.set_gpr32(instr.meta_data[0] as usize, result);     // write to rm = destination
    cpu.update_flags_add32(op1, op2, result);
}

/// ADD_EAX_Id: ADD EAX, imm32
/// Opcode: 0x05
/// Immediate value stored in operand_data.Id
pub fn ADD_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let eax = cpu.eax();
    let imm: u32 = instr.modrm_form.operand_data.id();
    let result = eax.wrapping_add(imm);

    cpu.set_eax(result);
    cpu.update_flags_add32(eax, imm, result);
}

/// ADD_EdId_R: ADD r32, imm32 (register form, sign-extended immediate)
/// Original: bochs/cpu/arith32.cc ADD_EdIdR
/// Opcode: 0x81/0x83, ModRM: r/m32, imm32/imm8
pub fn ADD_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_reg = instr.meta_data[0] as usize;
    let op1 = cpu.get_gpr32(dst_reg);
    let op2 = instr.id(); // Sign-extended immediate
    let result = op1.wrapping_add(op2);

    cpu.set_gpr32(dst_reg, result);
    cpu.update_flags_add32(op1, op2, result);
}

/// SUB_GdEd_R: SUB r32, r/m32 (register form)
/// Opcode: 0x2B, ModRM: r32, r/m32 (register)
pub fn SUB_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

    let dst_val = cpu.get_gpr32(dst_idx);
    let src_val = cpu.get_gpr32(src_idx);
    let result = dst_val.wrapping_sub(src_val);

    cpu.set_gpr32(dst_idx, result);
    cpu.update_flags_sub32(dst_val, src_val, result);
}

/// SUB_EdGd_R: SUB r/m32, r32 (register form)
/// Opcode: 0x29: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn SUB_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);  // rm = destination/first operand
    let op2 = cpu.get_gpr32(instr.meta_data[1] as usize);  // nnn = source/second operand
    let result = op1.wrapping_sub(op2);

    cpu.set_gpr32(instr.meta_data[0] as usize, result);     // write to rm = destination
    cpu.update_flags_sub32(op1, op2, result);
}

/// SUB_EAX_Id: SUB EAX, imm32
/// Opcode: 0x2D
pub fn SUB_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let eax = cpu.eax();
    let imm: u32 = instr.modrm_form.operand_data.id();
    let result = eax.wrapping_sub(imm);

    cpu.set_eax(result);
    cpu.update_flags_sub32(eax, imm, result);
}

/// SUB_EdId_R: SUB r32, imm32 (register form, sign-extended immediate)
/// Original: bochs/cpu/arith32.cc SUB_EdIdR
/// Opcode: 0x81/0x83, ModRM: r/m32, imm32/imm8
pub fn SUB_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_reg = instr.meta_data[0] as usize;
    let op1 = cpu.get_gpr32(dst_reg);
    let op2 = instr.id(); // Sign-extended immediate
    let result = op1.wrapping_sub(op2);

    cpu.set_gpr32(dst_reg, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// CMP_EdId_R: CMP r32, imm32 (register form, sign-extended immediate)
/// Original: bochs/cpu/arith32.cc CMP_EdIdR
/// Opcode: 0x81/0x83, ModRM: r/m32, imm32/imm8
pub fn CMP_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_reg = instr.meta_data[0] as usize;
    let op1 = cpu.get_gpr32(dst_reg);
    let op2 = instr.id(); // Sign-extended immediate
    let result = op1.wrapping_sub(op2);

    // CMP only sets flags, doesn't write result
    cpu.update_flags_sub32(op1, op2, result);
}

/// CMP_EdGd_R: CMP r/m32, r32 (register form)
/// Opcode: 0x39: decoder swaps for 16/32-bit store: [0]=rm=first operand, [1]=nnn=second operand
/// Performs rm - nnn and sets flags without storing result
fn CMP_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);  // rm = first operand
    let op2 = cpu.get_gpr32(instr.meta_data[1] as usize);  // nnn = second operand
    let diff = op1.wrapping_sub(op2);

    cpu.update_flags_sub32(op1, op2, diff);

    tracing::trace!("CMP r{}d, r{}d: {:#010x} - {:#010x}",
        instr.meta_data[0], instr.meta_data[1], op1, op2);
}

/// ADC_EdGd_R: ADC r/m32, r32 (register form)
/// Opcode: 0x11: decoder swaps for 16/32-bit store: [0]=rm=DEST, [1]=nnn=SOURCE
pub fn ADC_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);  // rm = destination/first operand
    let op2 = cpu.get_gpr32(instr.meta_data[1] as usize);  // nnn = source/second operand
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);

    cpu.set_gpr32(instr.meta_data[0] as usize, result);     // write to rm = destination
    cpu.update_flags_add32(op1, op2, result);
}

/// ADC_GdEd_R: ADC r32, r/m32 (register form)
/// Original: bochs/cpu/arith32.cc ADC_GdEd (register case)
/// Opcode: 0x13, ModRM: r32, r/m32 (register)
pub fn ADC_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst_idx = instr.meta_data[0] as usize;
    let src_idx = instr.meta_data[1] as usize;

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
/// Decoder swaps: src() = meta_data[1] = nnn = SOURCE register
pub fn ADD_EdGd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let result = op1.wrapping_add(op2);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADD_GdEd_M: ADD r32, r/m32 (memory form) - read memory, write register
pub fn ADD_GdEd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.read_virtual_dword(seg, eaddr)?;
    let result = op1.wrapping_add(op2);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADD_EdId_M: ADD r/m32, imm32 (memory form)
pub fn ADD_EdId_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let op2 = instr.id();
    let result = op1.wrapping_add(op2);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// SUB_EdGd_M: SUB r/m32, r32 (memory form)
/// Decoder swaps: src() = meta_data[1] = nnn = SOURCE register
pub fn SUB_EdGd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let result = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SUB_GdEd_M: SUB r32, r/m32 (memory form)
pub fn SUB_GdEd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.read_virtual_dword(seg, eaddr)?;
    let result = op1.wrapping_sub(op2);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SUB_EdId_M: SUB r/m32, imm32 (memory form)
pub fn SUB_EdId_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let op2 = instr.id();
    let result = op1.wrapping_sub(op2);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// CMP_EdGd_M: CMP r/m32, r32 (memory form)
/// Decoder swaps: src() = meta_data[1] = nnn = SOURCE register
pub fn CMP_EdGd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_virtual_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// CMP_GdEd_M: CMP r32, r/m32 (memory form)
pub fn CMP_GdEd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.read_virtual_dword(seg, eaddr)?;
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// CMP_EdId_M: CMP r/m32, imm32 (memory form)
pub fn CMP_EdId_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.read_virtual_dword(seg, eaddr)?;
    let op2 = instr.id();
    let result = op1.wrapping_sub(op2);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// ADC_EdGd_M: ADC r/m32, r32 (memory form)
/// Decoder swaps: src() = meta_data[1] = nnn = SOURCE register
pub fn ADC_EdGd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_add(op2).wrapping_add(cf);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_add32(op1, op2, result);
    Ok(())
}

/// ADC_GdEd_M: ADC r32, r/m32 (memory form)
pub fn ADC_GdEd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.read_virtual_dword(seg, eaddr)?;
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
pub fn ADD_EdGd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { ADD_EdGd_R(cpu, instr); Ok(()) } else { ADD_EdGd_M(cpu, instr) }
}

/// ADD r32, r/m32 - unified (Bochs: ADD_GdEdR / ADD_GdEdM)
pub fn ADD_GdEd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { ADD_GdEd_R(cpu, instr); Ok(()) } else { ADD_GdEd_M(cpu, instr) }
}

/// ADD r/m32, imm32 - unified (Bochs: ADD_EdIdR / ADD_EdIdM)
pub fn ADD_EdId<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { ADD_EdId_R(cpu, instr); Ok(()) } else { ADD_EdId_M(cpu, instr) }
}

/// SUB r/m32, r32 - unified (Bochs: SUB_EdGdR / SUB_EdGdM)
pub fn SUB_EdGd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { SUB_EdGd_R(cpu, instr); Ok(()) } else { SUB_EdGd_M(cpu, instr) }
}

/// SUB r32, r/m32 - unified (Bochs: SUB_GdEdR / SUB_GdEdM)
pub fn SUB_GdEd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { SUB_GdEd_R(cpu, instr); Ok(()) } else { SUB_GdEd_M(cpu, instr) }
}

/// SUB r/m32, imm32 - unified (Bochs: SUB_EdIdR / SUB_EdIdM)
pub fn SUB_EdId<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { SUB_EdId_R(cpu, instr); Ok(()) } else { SUB_EdId_M(cpu, instr) }
}

/// CMP r/m32, r32 - unified (Bochs: CMP_EdGdR / CMP_EdGdM)
pub fn CMP_EdGd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { CMP_EdGd_R(cpu, instr); Ok(()) } else { CMP_EdGd_M(cpu, instr) }
}

/// CMP r32, r/m32 - unified (Bochs: CMP_GdEdR / CMP_GdEdM)
pub fn CMP_GdEd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { CMP_EdGd_R(cpu, instr); Ok(()) } else { CMP_GdEd_M(cpu, instr) }
}

/// CMP r/m32, imm32 - unified (Bochs: CMP_EdIdR / CMP_EdIdM)
pub fn CMP_EdId<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { CMP_EdId_R(cpu, instr); Ok(()) } else { CMP_EdId_M(cpu, instr) }
}

/// ADC r/m32, r32 - unified (Bochs: ADC_EdGdR / ADC_EdGdM)
pub fn ADC_EdGd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { ADC_EdGd_R(cpu, instr); Ok(()) } else { ADC_EdGd_M(cpu, instr) }
}

/// ADC r32, r/m32 - unified (Bochs: ADC_GdEdR / ADC_GdEdM)
pub fn ADC_GdEd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { ADC_GdEd_R(cpu, instr); Ok(()) } else { ADC_GdEd_M(cpu, instr) }
}

// =========================================================================
// NEG - Two's Complement Negation (32-bit)
// Matching Bochs arith32.cc NEG_EdR / NEG_EdM
// =========================================================================

/// NEG r32 - Negate register (register form)
pub fn NEG_EdR<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let dst = instr.dst() as usize;
    let op1 = cpu.get_gpr32(dst);
    let result = 0u32.wrapping_sub(op1);
    cpu.set_gpr32(dst, result);
    cpu.update_flags_sub32(0, op1, result);
}

/// NEG m32 - Negate memory (memory form)
pub fn NEG_EdM<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let result = 0u32.wrapping_sub(op1);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_sub32(0, op1, result);
    Ok(())
}

/// NEG r/m32 - unified dispatch
pub fn NEG_Ed<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { NEG_EdR(cpu, instr); Ok(()) } else { NEG_EdM(cpu, instr) }
}

// =========================================================================
// SBB (Subtract with Borrow) - 32-bit
// =========================================================================

/// SBB r/m32, r32 - register form (opcode 0x19)
pub fn SBB_EdGd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);
    let op2 = cpu.get_gpr32(instr.meta_data[1] as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.meta_data[0] as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// SBB r/m32, r32 - memory form
pub fn SBB_EdGd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let (op1, laddr) = cpu.read_rmw_virtual_dword(seg, eaddr)?;
    let op2 = cpu.get_gpr32(instr.src() as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.write_rmw_linear_dword(laddr, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SBB r/m32, r32 - unified dispatch
pub fn SBB_EdGd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { SBB_EdGd_R(cpu, instr); Ok(()) } else { SBB_EdGd_M(cpu, instr) }
}

/// SBB r32, r/m32 - register form (opcode 0x1B)
pub fn SBB_GdEd_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);
    let op2 = cpu.get_gpr32(instr.meta_data[1] as usize);
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.meta_data[0] as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// SBB r32, r/m32 - memory form
pub fn SBB_GdEd_M<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    let eaddr = cpu.resolve_addr32(instr);
    let seg = BxSegregs::from(instr.seg());
    let op1 = cpu.get_gpr32(instr.dst() as usize);
    let op2 = cpu.read_virtual_dword(seg, eaddr)?;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.dst() as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
    Ok(())
}

/// SBB r32, r/m32 - unified dispatch
pub fn SBB_GdEd<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) -> Result<(), crate::cpu::CpuError> {
    if instr.mod_c0() { SBB_GdEd_R(cpu, instr); Ok(()) } else { SBB_GdEd_M(cpu, instr) }
}

/// SBB EAX, imm32 (opcode 0x1D)
pub fn SBB_EAX_Id<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let eax = cpu.eax();
    let imm = instr.modrm_form.operand_data.id();
    let cf = cpu.get_cf() as u32;
    let result = eax.wrapping_sub(imm).wrapping_sub(cf);
    cpu.set_eax(result);
    cpu.update_flags_sub32(eax, imm, result);
}

/// SBB r/m32, imm32 (opcode 0x81 /3) - register form
pub fn SBB_EdId_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);
    let op2 = instr.modrm_form.operand_data.id();
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.meta_data[0] as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}

/// SBB r/m32, imm8 sign-extended (opcode 0x83 /3) - register form
pub fn SBB_EdIb_R<I: BxCpuIdTrait>(cpu: &mut BxCpuC<I>, instr: &BxInstructionGenerated) {
    let op1 = cpu.get_gpr32(instr.meta_data[0] as usize);
    let op2 = instr.ib() as i8 as i32 as u32;
    let cf = cpu.get_cf() as u32;
    let result = op1.wrapping_sub(op2).wrapping_sub(cf);
    cpu.set_gpr32(instr.meta_data[0] as usize, result);
    cpu.update_flags_sub32(op1, op2, result);
}
