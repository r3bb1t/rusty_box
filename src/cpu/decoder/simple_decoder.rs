use crate::cpu::decoder::ia_opcodes::Opcode;
use crate::cpu::decoder::instr_generated::BxInstructionGenerated;

// Very small, focused decoder supporting a handful of 32-bit
// register-register and immediate forms for early boot code.
// It only decodes enough to populate `BxInstructionGenerated`'s
// opcode, `meta_info.ilen` and `meta_data` fields for dst/src
// register indices. Memory-modR/M (non-register) forms are not
// implemented here.
pub fn decode_simple_32(bytes: &[u8]) -> Option<BxInstructionGenerated> {
    if bytes.is_empty() {
        return None;
    }

    let b0 = bytes[0];
    let mut instr = BxInstructionGenerated::default();

    match b0 {
        0x90 => {
            instr.meta_info.ia_opcode = Opcode::Nop;
            instr.meta_info.ilen = 1;
            Some(instr)
        }
        // MOV r32, r/m32  (0x8B) -- we support reg-reg (mod==3)
        0x8B => {
            if bytes.len() < 2 {
                return None;
            }
            let modrm = bytes[1];
            let mod_bits = (modrm & 0b1100_0000) >> 6;
            let reg = ((modrm & 0b0011_1000) >> 3) as u8;
            let rm = (modrm & 0b0000_0111) as u8;
            if mod_bits == 0b11 {
                instr.meta_info.ia_opcode = Opcode::MovOp32GdEd; // MOV Gd, Ed (32-bit)
                instr.meta_info.ilen = 2;
                instr.meta_data[0] = reg;
                instr.meta_data[1] = rm;
                Some(instr)
            } else {
                None
            }
        }
        // MOV r/m32, r32 (0x89) -- support reg-reg
        0x89 => {
            if bytes.len() < 2 { return None; }
            let modrm = bytes[1];
            let mod_bits = (modrm & 0b1100_0000) >> 6;
            let reg = ((modrm & 0b0011_1000) >> 3) as u8;
            let rm = (modrm & 0b0000_0111) as u8;
            if mod_bits == 0b11 {
                instr.meta_info.ia_opcode = Opcode::MovOp32EdGd; // MOV Ed, Gd (32-bit)
                instr.meta_info.ilen = 2;
                instr.meta_data[0] = rm; // destination is r/m (here a reg)
                instr.meta_data[1] = reg;
                Some(instr)
            } else {
                None
            }
        }
        // MOV r32, imm32  (0xB8 + rd)
        0xB8..=0xBF => {
            let reg = (b0 - 0xB8) as u8;
            if bytes.len() < 5 { return None; }
            let imm = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            instr.meta_info.ia_opcode = Opcode::MovEdId; // Mov Ed, Id (use generic imm form)
            instr.meta_info.ilen = 5;
            instr.meta_data[0] = reg;
            // store immediate into operand_data.Id
            instr.modrm_form.operand_data = unsafe { core::mem::transmute(imm) };
            Some(instr)
        }
        // ADD Gd, Ed  (0x03) reg-reg
        0x03 => {
            if bytes.len() < 2 { return None; }
            let modrm = bytes[1];
            let mod_bits = (modrm & 0b1100_0000) >> 6;
            let reg = ((modrm & 0b0011_1000) >> 3) as u8;
            let rm = (modrm & 0b0000_0111) as u8;
            if mod_bits == 0b11 {
                instr.meta_info.ia_opcode = Opcode::AddGdEd;
                instr.meta_info.ilen = 2;
                instr.meta_data[0] = reg;
                instr.meta_data[1] = rm;
                Some(instr)
            } else { None }
        }
        // ADD Ev, Gv (0x01) reg-reg
        0x01 => {
            if bytes.len() < 2 { return None; }
            let modrm = bytes[1];
            let mod_bits = (modrm & 0b1100_0000) >> 6;
            let reg = ((modrm & 0b0011_1000) >> 3) as u8;
            let rm = (modrm & 0b0000_0111) as u8;
            if mod_bits == 0b11 {
                instr.meta_info.ia_opcode = Opcode::AddEdGd;
                instr.meta_info.ilen = 2;
                instr.meta_data[0] = rm;
                instr.meta_data[1] = reg;
                Some(instr)
            } else { None }
        }
        // ADD EAX, imm32 (0x05)
        0x05 => {
            if bytes.len() < 5 { return None; }
            let imm = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            instr.meta_info.ia_opcode = Opcode::AddEaxid;
            instr.meta_info.ilen = 5;
            instr.modrm_form.operand_data = unsafe { core::mem::transmute(imm) };
            Some(instr)
        }
        // SUB Gd, Ed (0x2B)
        0x2B => {
            if bytes.len() < 2 { return None; }
            let modrm = bytes[1];
            let mod_bits = (modrm & 0b1100_0000) >> 6;
            let reg = ((modrm & 0b0011_1000) >> 3) as u8;
            let rm = (modrm & 0b0000_0111) as u8;
            if mod_bits == 0b11 {
                instr.meta_info.ia_opcode = Opcode::SubGdEd;
                instr.meta_info.ilen = 2;
                instr.meta_data[0] = reg;
                instr.meta_data[1] = rm;
                Some(instr)
            } else { None }
        }
        // SUB Ev, Gv (0x29)
        0x29 => {
            if bytes.len() < 2 { return None; }
            let modrm = bytes[1];
            let mod_bits = (modrm & 0b1100_0000) >> 6;
            let reg = ((modrm & 0b0011_1000) >> 3) as u8;
            let rm = (modrm & 0b0000_0111) as u8;
            if mod_bits == 0b11 {
                instr.meta_info.ia_opcode = Opcode::SubEdGd;
                instr.meta_info.ilen = 2;
                instr.meta_data[0] = rm;
                instr.meta_data[1] = reg;
                Some(instr)
            } else { None }
        }
        // SUB EAX, imm32 (0x2D)
        0x2D => {
            if bytes.len() < 5 { return None; }
            let imm = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            instr.meta_info.ia_opcode = Opcode::SubEaxid;
            instr.meta_info.ilen = 5;
            instr.modrm_form.operand_data = unsafe { core::mem::transmute(imm) };
            Some(instr)
        }
        _ => None,
    }
}
