//! Const-compatible 64-bit instruction decoder
//!
//! This module provides a `const fn` instruction decoder for x86-64 mode
//! that returns `BxInstructionGenerated` - the same structure used by
//! the non-const decoder.

use super::error::{DecodeError, DecodeResult};
use super::fetchdecode_generated::BxDecodeError;
use super::ia_opcodes::Opcode;
use super::instr::MetaInfoFlags;
use super::instr_generated::{
    BxInstructionGenerated, BxInstructionMetaInfo, DisplacementData, ModRmForm, OperandData,
};
use super::BxSegregs;

// Import opcode tables
use super::fetchdecode_opmap::*;
use super::fetchdecode_opmap_0f38::BxOpcodeTable0F38;
use super::fetchdecode_opmap_0f3a::BxOpcodeTable0F3A;

// Metadata array indices
#[allow(dead_code)]
const BX_INSTR_METADATA_DST: usize = 0;
#[allow(dead_code)]
const BX_INSTR_METADATA_SRC1: usize = 1;
#[allow(dead_code)]
const BX_INSTR_METADATA_SRC2: usize = 2;
#[allow(dead_code)]
const BX_INSTR_METADATA_SRC3: usize = 3;
const BX_INSTR_METADATA_SEG: usize = 4;
const BX_INSTR_METADATA_BASE: usize = 5;
const BX_INSTR_METADATA_INDEX: usize = 6;
const BX_INSTR_METADATA_SCALE: usize = 7;

// Decoding mask bit offsets (from fetchdecode_generated.rs)
const OS64_OFFSET: u32 = 23;
const OS32_OFFSET: u32 = 22;
const AS64_OFFSET: u32 = 21;
const AS32_OFFSET: u32 = 20;
const SSE_PREFIX_OFFSET: u32 = 18;
const MODC0_OFFSET: u32 = 16;
const IS64_OFFSET: u32 = 15;
const RRR_OFFSET: u32 = 4;
const NNN_OFFSET: u32 = 0;

/// Search opcode table for matching opcode
const fn find_opcode_in_table(table: &[u64], decmask: u32) -> Opcode {
    let mut i = 0;
    while i < table.len() {
        let entry = table[i];
        let ignmsk = (entry & 0xFFFFFF) as u32;
        let opmsk = ((entry >> 24) & 0xFFFFFF) as u32;

        if (opmsk & ignmsk) == (decmask & ignmsk) {
            let opcode_raw = ((entry >> 48) & 0x7FFF) as u16;
            return Opcode::from_u16_const(opcode_raw);
        }
        i += 1;
    }
    Opcode::IaError
}

/// Const-compatible 64-bit instruction decoder
///
/// Decodes an x86-64 instruction and returns a `Result` containing either
/// a `BxInstructionGenerated` struct on success, or a `DecodeError` on failure.
/// This is the const fn equivalent of `fetch_decode64`.
pub const fn const_fetch_decode64(bytes: &[u8]) -> DecodeResult<BxInstructionGenerated> {
    let mut instr = BxInstructionGenerated {
        meta_info: BxInstructionMetaInfo {
            ia_opcode: Opcode::IaError,
            ilen: 0,
            metainfo1: MetaInfoFlags::empty(),
        },
        meta_data: [0u8; 8],
        modrm_form: ModRmForm {
            operand_data: OperandData { id: 0 },
            displacement: DisplacementData {
                data32: 0,
                data16: 0,
            },
        },
    };

    if bytes.is_empty() {
        return Err(DecodeError::BufferUnderflow);
    }

    let max_len = if bytes.len() > 15 { 15 } else { bytes.len() };
    let mut pos = 0usize;

    // Initialize for 64-bit mode: os32=1, as32=1, os64=0, as64=1
    let mut metainfo1_bits: u8 =
        MetaInfoFlags::Os32.bits() | MetaInfoFlags::As32.bits() | MetaInfoFlags::As64.bits();

    // REX prefix tracking
    let mut rex_prefix: u8 = 0;
    let mut sse_prefix: u8 = 0; // 0=none, 1=66, 2=F2, 3=F3
    let mut seg_override: u8 = 7; // 7 = none

    // === Phase 1: Parse legacy prefixes ===
    // Per original C++: legacy prefixes reset any previously seen REX prefix
    while pos < max_len {
        let b = bytes[pos];
        match b {
            // Segment overrides (ES, CS, SS, DS ignored in 64-bit for most; FS/GS valid)
            // In 64-bit mode, CS:, DS:, ES:, SS: are ignored but reset REX
            0x26 | 0x2E | 0x36 | 0x3E => {
                rex_prefix = 0; // Reset REX prefix
                                // These segment overrides are ignored in 64-bit mode
            }
            0x64 => {
                rex_prefix = 0;
                seg_override = 4; // FS - valid in 64-bit
            }
            0x65 => {
                rex_prefix = 0;
                seg_override = 5; // GS - valid in 64-bit
            }

            // Operand size override
            0x66 => {
                rex_prefix = 0;
                metainfo1_bits &= !MetaInfoFlags::Os32.bits();
                if sse_prefix == 0 {
                    sse_prefix = 1;
                }
            }

            // Address size override
            0x67 => {
                rex_prefix = 0;
                metainfo1_bits &= !MetaInfoFlags::As32.bits();
                metainfo1_bits &= !MetaInfoFlags::As64.bits();
            }

            // LOCK prefix
            0xF0 => {
                rex_prefix = 0;
                metainfo1_bits = (metainfo1_bits & 0x3F) | (1 << 6);
            }

            // REPNE/REPNZ (also SSE prefix)
            0xF2 => {
                rex_prefix = 0;
                metainfo1_bits = (metainfo1_bits & 0x3F) | (2 << 6);
                sse_prefix = 2;
            }

            // REP/REPE/REPZ (also SSE prefix)
            0xF3 => {
                rex_prefix = 0;
                metainfo1_bits = (metainfo1_bits & 0x3F) | (3 << 6);
                sse_prefix = 3;
            }

            // REX prefixes (0x40-0x4F)
            0x40..=0x4F => {
                rex_prefix = b & 0x0F;
                // REX.W sets 64-bit operand size
                if (rex_prefix & 0x08) != 0 {
                    metainfo1_bits |= MetaInfoFlags::Os64.bits();
                }
                pos += 1;
                break;
            }

            _ => break,
        }
        pos += 1;
    }

    if pos >= max_len {
        return Err(DecodeError::PrefixBufferUnderflow);
    }

    // Set segment override
    if seg_override < 7 {
        instr.meta_data[BX_INSTR_METADATA_SEG] = seg_override;
    } else {
        instr.meta_data[BX_INSTR_METADATA_SEG] = BxSegregs::Ds as u8;
    }

    // === Phase 2: Parse opcode ===
    let mut b1 = bytes[pos] as u32;
    pos += 1;

    // Check for VEX/EVEX/XOP prefixes
    if b1 == 0xC4 || b1 == 0xC5 {
        // VEX prefix - simplified handling
        if pos < max_len && (bytes[pos] & 0xC0) == 0xC0 {
            // This is a VEX prefix (mod=11 check)
            return Err(DecodeError::Decoder(BxDecodeError::BxIllegalVexXopOpcodeMap)); // VEX not fully supported in const
        }
    }

    if b1 == 0x62 {
        // EVEX prefix - simplified handling
        if pos + 2 < max_len && (bytes[pos] & 0x0C) == 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet)); // EVEX not fully supported in const
        }
    }

    // Two-byte escape (0F xx)
    let mut opcode_map: u8 = 0; // 0=1-byte, 1=0F, 2=0F38, 3=0F3A
    if b1 == 0x0F {
        if pos >= max_len {
            return Err(DecodeError::OpcodeBufferUnderflow);
        }

        let b2 = bytes[pos];
        pos += 1;

        if b2 == 0x38 {
            // 0F 38 xx
            if pos >= max_len {
                return Err(DecodeError::OpcodeBufferUnderflow);
            }
            b1 = 0x200 | (bytes[pos] as u32);
            opcode_map = 2;
            pos += 1;
        } else if b2 == 0x3A {
            // 0F 3A xx
            if pos >= max_len {
                return Err(DecodeError::OpcodeBufferUnderflow);
            }
            b1 = 0x300 | (bytes[pos] as u32);
            opcode_map = 3;
            pos += 1;
        } else if b2 == 0x0F {
            // 3DNow! (0F 0F) - handle specially
            opcode_map = 1;
            b1 = 0x10F; // 3DNow marker
        } else {
            b1 = 0x100 | (b2 as u32);
            opcode_map = 1;
        }
    }

    // === Phase 3: Parse ModRM if needed ===
    let needs_modrm = opcode_needs_modrm_64(b1, opcode_map);

    let mut nnn: u32 = (b1 >> 3) & 0x7;
    let mut rm: u32 = b1 & 0x7;

    if needs_modrm {
        if pos >= max_len {
            return Err(DecodeError::ModRmBufferUnderflow);
        }

        let modrm = bytes[pos];
        pos += 1;

        let mod_field = (modrm >> 6) & 0x3;
        nnn = ((modrm >> 3) & 0x7) as u32;
        rm = (modrm & 0x7) as u32;

        // REX extensions
        if (rex_prefix & 0x04) != 0 {
            nnn |= 8;
        } // REX.R
        if (rex_prefix & 0x01) != 0 {
            rm |= 8;
        } // REX.B

        if mod_field == 3 {
            // Register mode
            metainfo1_bits |= MetaInfoFlags::ModC0.bits();
        } else {
            // Memory mode
            let use_sib = (rm & 0x7) == 4;

            if use_sib {
                if pos >= max_len {
                    return Err(DecodeError::SibBufferUnderflow);
                }

                let sib = bytes[pos];
                pos += 1;

                let scale = (sib >> 6) & 0x3;
                let mut index = ((sib >> 3) & 0x7) as u8;
                let mut base = (sib & 0x7) as u8;

                // REX extensions
                if (rex_prefix & 0x02) != 0 {
                    index |= 8;
                } // REX.X
                if (rex_prefix & 0x01) != 0 {
                    base |= 8;
                } // REX.B

                instr.meta_data[BX_INSTR_METADATA_SCALE] = scale;
                instr.meta_data[BX_INSTR_METADATA_INDEX] = index;
                instr.meta_data[BX_INSTR_METADATA_BASE] = base;

                // Displacement for SIB
                if mod_field == 0 && (base & 0x7) == 5 {
                    // [disp32] or [base+disp32]
                    if pos + 4 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u32_le(bytes, pos);
                    pos += 4;
                    instr.modrm_form.displacement.data32 = disp;
                    instr.meta_data[BX_INSTR_METADATA_BASE] = 19; // BX_NIL_REGISTER
                }
            } else {
                instr.meta_data[BX_INSTR_METADATA_BASE] = (rm & 0xF) as u8;
                instr.meta_data[BX_INSTR_METADATA_INDEX] = 4; // No index

                // Check for RIP-relative (mod=0, rm=5)
                if mod_field == 0 && (rm & 0x7) == 5 {
                    // RIP-relative addressing
                    if pos + 4 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u32_le(bytes, pos);
                    pos += 4;
                    instr.modrm_form.displacement.data32 = disp;
                    instr.meta_data[BX_INSTR_METADATA_BASE] = 17; // BX_64BIT_REG_RIP
                }
            }

            // Handle displacement based on mod field
            if mod_field == 1 {
                // disp8
                if pos >= max_len {
                    return Err(DecodeError::DisplacementBufferUnderflow);
                }
                let disp = bytes[pos] as i8 as i32 as u32;
                pos += 1;
                instr.modrm_form.displacement.data32 = disp;
            } else if mod_field == 2 {
                // disp32
                if pos + 4 > max_len {
                    return Err(DecodeError::DisplacementBufferUnderflow);
                }
                let disp = read_u32_le(bytes, pos);
                pos += 4;
                instr.modrm_form.displacement.data32 = disp;
            }
        }
    } else {
        // No ModRM - instruction uses register encoded in opcode
        metainfo1_bits |= MetaInfoFlags::ModC0.bits();
    }

    // Store register fields
    instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;
    instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;

    // === Phase 4: Parse immediate ===
    let imm_size = get_immediate_size_64(b1, opcode_map, sse_prefix, metainfo1_bits);

    if imm_size > 0 {
        if pos + (imm_size as usize) > max_len {
            return Err(DecodeError::ImmediateBufferUnderflow);
        }

        match imm_size {
            1 => {
                instr.modrm_form.operand_data.id = bytes[pos] as u32;
                pos += 1;
            }
            2 => {
                instr.modrm_form.operand_data.id = read_u16_le(bytes, pos) as u32;
                pos += 2;
            }
            4 => {
                instr.modrm_form.operand_data.id = read_u32_le(bytes, pos);
                pos += 4;
            }
            8 => {
                // 64-bit immediate (MOV reg, imm64)
                // Store in displacement fields as we don't have iq field directly
                instr.modrm_form.operand_data.id = read_u32_le(bytes, pos);
                instr.modrm_form.displacement.data32 = read_u32_le(bytes, pos + 4);
                pos += 8;
            }
            _ => {}
        }
    }

    // Finalize instruction
    instr.meta_info.ilen = pos as u8;
    instr.meta_info.metainfo1 = MetaInfoFlags::from_bits_retain(metainfo1_bits);

    // Build decmask for opcode lookup
    let mod_c0 = (metainfo1_bits & MetaInfoFlags::ModC0.bits()) != 0;
    let os64 = (metainfo1_bits & MetaInfoFlags::Os64.bits()) != 0;
    let os32 = (metainfo1_bits & MetaInfoFlags::Os32.bits()) != 0;
    let as64 = (metainfo1_bits & MetaInfoFlags::As64.bits()) != 0;
    let as32 = (metainfo1_bits & MetaInfoFlags::As32.bits()) != 0;

    // Only include nnn/rm in decmask if instruction has ModRM
    // For instructions without ModRM, the nnn/rm values derived from opcode bits
    // shouldn't affect opcode table lookup
    let decmask: u32 = (if os64 { 1 } else { 0 } << OS64_OFFSET)
        | (if os32 { 1 } else { 0 } << OS32_OFFSET)
        | (if as64 { 1 } else { 0 } << AS64_OFFSET)
        | (if as32 { 1 } else { 0 } << AS32_OFFSET)
        | ((sse_prefix as u32) << SSE_PREFIX_OFFSET)
        | (if mod_c0 { 1 } else { 0 } << MODC0_OFFSET)
        | (1 << IS64_OFFSET) // 64-bit mode
        | if needs_modrm { ((rm & 0x7) << RRR_OFFSET) | ((nnn & 0x7) << NNN_OFFSET) } else { 0 };

    // Look up opcode from tables
    instr.meta_info.ia_opcode = lookup_opcode_64(b1, opcode_map, decmask, nnn);

    // Check if opcode lookup failed
    if matches!(instr.meta_info.ia_opcode, Opcode::IaError) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    Ok(instr)
}

/// Get opcode table and look up opcode for 64-bit mode
const fn lookup_opcode_64(b1: u32, opcode_map: u8, decmask: u32, _nnn: u32) -> Opcode {
    if opcode_map == 0 {
        // One-byte opcodes
        let table = get_opcode_table_64(b1 as u8);
        if table.is_empty() {
            return Opcode::IaError;
        }
        find_opcode_in_table(table, decmask)
    } else if opcode_map == 1 {
        // Two-byte opcodes (0F xx)
        let table = get_opcode_table_0f_64((b1 & 0xFF) as u8);
        if table.is_empty() {
            return Opcode::IaError;
        }
        find_opcode_in_table(table, decmask)
    } else if opcode_map == 2 {
        // Three-byte opcodes (0F 38 xx)
        let opcode = (b1 & 0xFF) as usize;
        if opcode < BxOpcodeTable0F38.len() {
            let table = BxOpcodeTable0F38[opcode];
            if table.is_empty() {
                Opcode::IaError
            } else {
                find_opcode_in_table(table, decmask)
            }
        } else {
            Opcode::IaError
        }
    } else if opcode_map == 3 {
        // Three-byte opcodes (0F 3A xx)
        let opcode = (b1 & 0xFF) as usize;
        if opcode < BxOpcodeTable0F3A.len() {
            let table = BxOpcodeTable0F3A[opcode];
            if table.is_empty() {
                Opcode::IaError
            } else {
                find_opcode_in_table(table, decmask)
            }
        } else {
            Opcode::IaError
        }
    } else {
        Opcode::IaError
    }
}

/// Get opcode table for one-byte opcode in 64-bit mode
const fn get_opcode_table_64(b1: u8) -> &'static [u64] {
    match b1 {
        0x00 => &BxOpcodeTable00,
        0x01 => &BxOpcodeTable01,
        0x02 => &BxOpcodeTable02,
        0x03 => &BxOpcodeTable03,
        0x04 => &BxOpcodeTable04,
        0x05 => &BxOpcodeTable05,
        0x06 => &BxOpcodeTable06,
        0x07 => &BxOpcodeTable07,
        0x08 => &BxOpcodeTable08,
        0x09 => &BxOpcodeTable09,
        0x0A => &BxOpcodeTable0A,
        0x0B => &BxOpcodeTable0B,
        0x0C => &BxOpcodeTable0C,
        0x0D => &BxOpcodeTable0D,
        0x0E => &BxOpcodeTable0E,
        0x10 => &BxOpcodeTable10,
        0x11 => &BxOpcodeTable11,
        0x12 => &BxOpcodeTable12,
        0x13 => &BxOpcodeTable13,
        0x14 => &BxOpcodeTable14,
        0x15 => &BxOpcodeTable15,
        0x16 => &BxOpcodeTable16,
        0x17 => &BxOpcodeTable17,
        0x18 => &BxOpcodeTable18,
        0x19 => &BxOpcodeTable19,
        0x1A => &BxOpcodeTable1A,
        0x1B => &BxOpcodeTable1B,
        0x1C => &BxOpcodeTable1C,
        0x1D => &BxOpcodeTable1D,
        0x1E => &BxOpcodeTable1E,
        0x1F => &BxOpcodeTable1F,
        0x20 => &BxOpcodeTable20,
        0x21 => &BxOpcodeTable21,
        0x22 => &BxOpcodeTable22,
        0x23 => &BxOpcodeTable23,
        0x24 => &BxOpcodeTable24,
        0x25 => &BxOpcodeTable25,
        0x27 => &BxOpcodeTable27,
        0x28 => &BxOpcodeTable28,
        0x29 => &BxOpcodeTable29,
        0x2A => &BxOpcodeTable2A,
        0x2B => &BxOpcodeTable2B,
        0x2C => &BxOpcodeTable2C,
        0x2D => &BxOpcodeTable2D,
        0x2F => &BxOpcodeTable2F,
        0x30 => &BxOpcodeTable30,
        0x31 => &BxOpcodeTable31,
        0x32 => &BxOpcodeTable32,
        0x33 => &BxOpcodeTable33,
        0x34 => &BxOpcodeTable34,
        0x35 => &BxOpcodeTable35,
        0x37 => &BxOpcodeTable37,
        0x38 => &BxOpcodeTable38,
        0x39 => &BxOpcodeTable39,
        0x3A => &BxOpcodeTable3A,
        0x3B => &BxOpcodeTable3B,
        0x3C => &BxOpcodeTable3C,
        0x3D => &BxOpcodeTable3D,
        0x3F => &BxOpcodeTable3F,
        0x40..=0x47 => &BxOpcodeTable40x47,
        0x48..=0x4F => &BxOpcodeTable48x4F,
        0x50..=0x57 => &BxOpcodeTable50x57,
        0x58..=0x5F => &BxOpcodeTable58x5F,
        0x60 => &BxOpcodeTable60,
        0x61 => &BxOpcodeTable61,
        0x63 => &BxOpcodeTable63_64,
        0x68 => &BxOpcodeTable68,
        0x69 => &BxOpcodeTable69,
        0x6A => &BxOpcodeTable6A,
        0x6B => &BxOpcodeTable6B,
        0x6C => &BxOpcodeTable6C,
        0x6D => &BxOpcodeTable6D,
        0x6E => &BxOpcodeTable6E,
        0x6F => &BxOpcodeTable6F,
        0x70 => &BxOpcodeTable70_64,
        0x71 => &BxOpcodeTable71_64,
        0x72 => &BxOpcodeTable72_64,
        0x73 => &BxOpcodeTable73_64,
        0x74 => &BxOpcodeTable74_64,
        0x75 => &BxOpcodeTable75_64,
        0x76 => &BxOpcodeTable76_64,
        0x77 => &BxOpcodeTable77_64,
        0x78 => &BxOpcodeTable78_64,
        0x79 => &BxOpcodeTable79_64,
        0x7A => &BxOpcodeTable7A_64,
        0x7B => &BxOpcodeTable7B_64,
        0x7C => &BxOpcodeTable7C_64,
        0x7D => &BxOpcodeTable7D_64,
        0x7E => &BxOpcodeTable7E_64,
        0x7F => &BxOpcodeTable7F_64,
        0x80 => &BxOpcodeTable80,
        0x81 => &BxOpcodeTable81,
        0x83 => &BxOpcodeTable83,
        0x84 => &BxOpcodeTable84,
        0x85 => &BxOpcodeTable85,
        0x86 => &BxOpcodeTable86,
        0x87 => &BxOpcodeTable87,
        0x88 => &BxOpcodeTable88,
        0x89 => &BxOpcodeTable89,
        0x8A => &BxOpcodeTable8A,
        0x8B => &BxOpcodeTable8B,
        0x8C => &BxOpcodeTable8C,
        0x8D => &BxOpcodeTable8D,
        0x8E => &BxOpcodeTable8E,
        0x8F => &BxOpcodeTable8F,
        0x90 => &BxOpcodeTable90x97,
        0x91..=0x97 => &BxOpcodeTable90x97,
        0x98 => &BxOpcodeTable98,
        0x99 => &BxOpcodeTable99,
        0x9B => &BxOpcodeTable9B,
        0x9C => &BxOpcodeTable9C,
        0x9D => &BxOpcodeTable9D,
        0x9E => &BxOpcodeTable9E_64,
        0x9F => &BxOpcodeTable9F_64,
        0xA0 => &BxOpcodeTableA0_64,
        0xA1 => &BxOpcodeTableA1_64,
        0xA2 => &BxOpcodeTableA2_64,
        0xA3 => &BxOpcodeTableA3_64,
        0xA4 => &BxOpcodeTableA4,
        0xA5 => &BxOpcodeTableA5,
        0xA6 => &BxOpcodeTableA6,
        0xA7 => &BxOpcodeTableA7,
        0xA8 => &BxOpcodeTableA8,
        0xA9 => &BxOpcodeTableA9,
        0xAA => &BxOpcodeTableAA,
        0xAB => &BxOpcodeTableAB,
        0xAC => &BxOpcodeTableAC,
        0xAD => &BxOpcodeTableAD,
        0xAE => &BxOpcodeTableAE,
        0xAF => &BxOpcodeTableAF,
        0xB0..=0xB7 => &BxOpcodeTableB0xB7,
        0xB8..=0xBF => &BxOpcodeTableB8xBF,
        0xC0 => &BxOpcodeTableC0,
        0xC1 => &BxOpcodeTableC1,
        0xC2 => &BxOpcodeTableC2_64,
        0xC3 => &BxOpcodeTableC3_64,
        0xC6 => &BxOpcodeTableC6,
        0xC7 => &BxOpcodeTableC7,
        0xC8 => &BxOpcodeTableC8_64,
        0xC9 => &BxOpcodeTableC9_64,
        0xCA => &BxOpcodeTableCA,
        0xCB => &BxOpcodeTableCB,
        0xCC => &BxOpcodeTableCC,
        0xCD => &BxOpcodeTableCD,
        0xCF => &BxOpcodeTableCF_64,
        0xD0 => &BxOpcodeTableD0,
        0xD1 => &BxOpcodeTableD1,
        0xD2 => &BxOpcodeTableD2,
        0xD3 => &BxOpcodeTableD3,
        0xD4 => &BxOpcodeTableD4,
        0xD5 => &BxOpcodeTableD5,
        0xD6 => &BxOpcodeTableD6,
        0xD7 => &BxOpcodeTableD7,
        0xE0 => &BxOpcodeTableE0_64,
        0xE1 => &BxOpcodeTableE1_64,
        0xE2 => &BxOpcodeTableE2_64,
        0xE3 => &BxOpcodeTableE3_64,
        0xE4 => &BxOpcodeTableE4,
        0xE5 => &BxOpcodeTableE5,
        0xE6 => &BxOpcodeTableE6,
        0xE7 => &BxOpcodeTableE7,
        0xE8 => &BxOpcodeTableE8_64,
        0xE9 => &BxOpcodeTableE9_64,
        0xEB => &BxOpcodeTableEB_64,
        0xEC => &BxOpcodeTableEC,
        0xED => &BxOpcodeTableED,
        0xEE => &BxOpcodeTableEE,
        0xEF => &BxOpcodeTableEF,
        0xF1 => &BxOpcodeTableF1,
        0xF4 => &BxOpcodeTableF4,
        0xF5 => &BxOpcodeTableF5,
        0xF6 => &BxOpcodeTableF6,
        0xF7 => &BxOpcodeTableF7,
        0xF8 => &BxOpcodeTableF8,
        0xF9 => &BxOpcodeTableF9,
        0xFA => &BxOpcodeTableFA,
        0xFB => &BxOpcodeTableFB,
        0xFC => &BxOpcodeTableFC,
        0xFD => &BxOpcodeTableFD,
        0xFE => &BxOpcodeTableFE,
        0xFF => &BxOpcodeTableFF,
        _ => &[],
    }
}

/// Get opcode table for two-byte opcode (0F xx) in 64-bit mode
const fn get_opcode_table_0f_64(b2: u8) -> &'static [u64] {
    match b2 {
        0x00 => &BxOpcodeTable0F00,
        0x01 => &BxOpcodeTable0F01,
        0x02 => &BxOpcodeTable0F02,
        0x03 => &BxOpcodeTable0F03,
        0x05 => &BxOpcodeTable0F05_64,
        0x06 => &BxOpcodeTable0F06,
        0x07 => &BxOpcodeTable0F07_64,
        0x08 => &BxOpcodeTable0F08,
        0x09 => &BxOpcodeTable0F09,
        0x0B => &BxOpcodeTable0F0B,
        0x0D => &BxOpcodeTable0F0D,
        0x0E => &BxOpcodeTable0F0E,
        0x10 => &BxOpcodeTable0F10,
        0x11 => &BxOpcodeTable0F11,
        0x12 => &BxOpcodeTable0F12,
        0x13 => &BxOpcodeTable0F13,
        0x14 => &BxOpcodeTable0F14,
        0x15 => &BxOpcodeTable0F15,
        0x16 => &BxOpcodeTable0F16,
        0x17 => &BxOpcodeTable0F17,
        0x18 => &BxOpcodeTable0F18,
        0x1E => &BxOpcodeTable0F1E,
        0x20 => &BxOpcodeTable0F20_64,
        0x21 => &BxOpcodeTable0F21_64,
        0x22 => &BxOpcodeTable0F22_64,
        0x23 => &BxOpcodeTable0F23_64,
        0x28 => &BxOpcodeTable0F28,
        0x29 => &BxOpcodeTable0F29,
        0x2A => &BxOpcodeTable0F2A,
        0x2B => &BxOpcodeTable0F2B,
        0x2C => &BxOpcodeTable0F2C,
        0x2D => &BxOpcodeTable0F2D,
        0x2E => &BxOpcodeTable0F2E,
        0x2F => &BxOpcodeTable0F2F,
        0x30 => &BxOpcodeTable0F30,
        0x31 => &BxOpcodeTable0F31,
        0x32 => &BxOpcodeTable0F32,
        0x33 => &BxOpcodeTable0F33,
        0x34 => &BxOpcodeTable0F34,
        0x35 => &BxOpcodeTable0F35,
        0x37 => &BxOpcodeTable0F37,
        0x40 => &BxOpcodeTable0F40,
        0x41 => &BxOpcodeTable0F41,
        0x42 => &BxOpcodeTable0F42,
        0x43 => &BxOpcodeTable0F43,
        0x44 => &BxOpcodeTable0F44,
        0x45 => &BxOpcodeTable0F45,
        0x46 => &BxOpcodeTable0F46,
        0x47 => &BxOpcodeTable0F47,
        0x48 => &BxOpcodeTable0F48,
        0x49 => &BxOpcodeTable0F49,
        0x4A => &BxOpcodeTable0F4A,
        0x4B => &BxOpcodeTable0F4B,
        0x4C => &BxOpcodeTable0F4C,
        0x4D => &BxOpcodeTable0F4D,
        0x4E => &BxOpcodeTable0F4E,
        0x4F => &BxOpcodeTable0F4F,
        0x80 => &BxOpcodeTable0F80_64,
        0x81 => &BxOpcodeTable0F81_64,
        0x82 => &BxOpcodeTable0F82_64,
        0x83 => &BxOpcodeTable0F83_64,
        0x84 => &BxOpcodeTable0F84_64,
        0x85 => &BxOpcodeTable0F85_64,
        0x86 => &BxOpcodeTable0F86_64,
        0x87 => &BxOpcodeTable0F87_64,
        0x88 => &BxOpcodeTable0F88_64,
        0x89 => &BxOpcodeTable0F89_64,
        0x8A => &BxOpcodeTable0F8A_64,
        0x8B => &BxOpcodeTable0F8B_64,
        0x8C => &BxOpcodeTable0F8C_64,
        0x8D => &BxOpcodeTable0F8D_64,
        0x8E => &BxOpcodeTable0F8E_64,
        0x8F => &BxOpcodeTable0F8F_64,
        0x90 => &BxOpcodeTable0F90,
        0x91 => &BxOpcodeTable0F91,
        0x92 => &BxOpcodeTable0F92,
        0x93 => &BxOpcodeTable0F93,
        0x94 => &BxOpcodeTable0F94,
        0x95 => &BxOpcodeTable0F95,
        0x96 => &BxOpcodeTable0F96,
        0x97 => &BxOpcodeTable0F97,
        0x98 => &BxOpcodeTable0F98,
        0x99 => &BxOpcodeTable0F99,
        0x9A => &BxOpcodeTable0F9A,
        0x9B => &BxOpcodeTable0F9B,
        0x9C => &BxOpcodeTable0F9C,
        0x9D => &BxOpcodeTable0F9D,
        0x9E => &BxOpcodeTable0F9E,
        0x9F => &BxOpcodeTable0F9F,
        0xA0 => &BxOpcodeTable0FA0,
        0xA1 => &BxOpcodeTable0FA1,
        0xA2 => &BxOpcodeTable0FA2,
        0xA3 => &BxOpcodeTable0FA3,
        0xA4 => &BxOpcodeTable0FA4,
        0xA5 => &BxOpcodeTable0FA5,
        0xA8 => &BxOpcodeTable0FA8,
        0xA9 => &BxOpcodeTable0FA9,
        0xAA => &BxOpcodeTable0FAA,
        0xAB => &BxOpcodeTable0FAB,
        0xAC => &BxOpcodeTable0FAC,
        0xAD => &BxOpcodeTable0FAD,
        0xAE => &BxOpcodeTable0FAE,
        0xAF => &BxOpcodeTable0FAF,
        0xB0 => &BxOpcodeTable0FB0,
        0xB1 => &BxOpcodeTable0FB1,
        0xB2 => &BxOpcodeTable0FB2,
        0xB3 => &BxOpcodeTable0FB3,
        0xB4 => &BxOpcodeTable0FB4,
        0xB5 => &BxOpcodeTable0FB5,
        0xB6 => &BxOpcodeTable0FB6,
        0xB7 => &BxOpcodeTable0FB7,
        0xB8 => &BxOpcodeTable0FB8,
        0xB9 => &BxOpcodeTable0FB9,
        0xBA => &BxOpcodeTable0FBA,
        0xBB => &BxOpcodeTable0FBB,
        0xBC => &BxOpcodeTable0FBC,
        0xBD => &BxOpcodeTable0FBD,
        0xBE => &BxOpcodeTable0FBE,
        0xBF => &BxOpcodeTable0FBF,
        0xC0 => &BxOpcodeTable0FC0,
        0xC1 => &BxOpcodeTable0FC1,
        0xC2 => &BxOpcodeTable0FC2,
        0xC3 => &BxOpcodeTable0FC3,
        0xC4 => &BxOpcodeTable0FC4,
        0xC5 => &BxOpcodeTable0FC5,
        0xC6 => &BxOpcodeTable0FC6,
        0xC7 => &BxOpcodeTable0FC7,
        0xC8..=0xCF => &BxOpcodeTable0FC8x0FCF,
        0xFF => &BxOpcodeTable0FFF,
        _ => &[],
    }
}

/// Check if opcode needs ModRM byte (64-bit mode)
const fn opcode_needs_modrm_64(b1: u32, map: u8) -> bool {
    if map == 0 {
        // One-byte opcodes
        let opcode = b1 as u8;
        !matches!(opcode,
            0x04 | 0x05 | 0x0C | 0x0D | 0x14 | 0x15 | 0x1C | 0x1D |
            0x24 | 0x25 | 0x2C | 0x2D | 0x34 | 0x35 | 0x3C | 0x3D |
            0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F |
            0x27 | 0x2F | 0x37 | 0x3F |
            0x40..=0x5F |
            0x60..=0x62 | 0x68 | 0x6A |
            0x70..=0x7F |
            0x90..=0x9F |
            0xA0..=0xAF |
            0xB0..=0xBF |
            0xC2 | 0xC3 | 0xC8 | 0xCA | 0xCB | 0xCC..=0xCF |
            0xD4..=0xD7 |
            0xE0..=0xEF |
            0xF1 | 0xF4 | 0xF5 | 0xF8..=0xFD
        )
    } else if map == 1 {
        // 0F map
        let opcode = (b1 & 0xFF) as u8;
        !matches!(opcode,
            0x05..=0x09 | 0x0B |
            0x30..=0x37 |
            0x77 |
            0x80..=0x8F |
            0xA0..=0xA2 | 0xA8..=0xAA |
            0xC8..=0xCF
        )
    } else {
        // 0F38, 0F3A maps always need ModRM
        true
    }
}

/// Get immediate size for opcode (64-bit mode)
const fn get_immediate_size_64(b1: u32, map: u8, _sse_prefix: u8, metainfo1: u8) -> u8 {
    let os32 = (metainfo1 & MetaInfoFlags::Os32.bits()) != 0;
    let os64 = (metainfo1 & MetaInfoFlags::Os64.bits()) != 0;

    if map == 0 {
        let opcode = b1 as u8;
        match opcode {
            // Ib
            0x04
            | 0x0C
            | 0x14
            | 0x1C
            | 0x24
            | 0x2C
            | 0x34
            | 0x3C
            | 0x6A
            | 0x6B
            | 0xA8
            | 0xB0..=0xB7
            | 0xCD
            | 0xD4
            | 0xD5
            | 0xE0..=0xE7
            | 0xEB
            | 0x70..=0x7F
            | 0x80
            | 0x82
            | 0x83
            | 0xC0
            | 0xC1
            | 0xC6 => 1,

            // Iw
            0xC2 | 0xCA => 2,

            // Iv/Id (operand-size dependent)
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D | 0x68 | 0x69 | 0xA9 | 0xE8
            | 0xE9 | 0x81 | 0xC7 => {
                if os32 {
                    4
                } else {
                    2
                }
            }

            // MOV reg, imm (B8-BF) - Iv or Iq
            0xB8..=0xBF => {
                if os64 {
                    8
                } else if os32 {
                    4
                } else {
                    2
                }
            }

            // ENTER: Iw + Ib = 3 bytes
            0xC8 => 3,

            _ => 0,
        }
    } else if map == 1 {
        let opcode = (b1 & 0xFF) as u8;
        match opcode {
            // Jcc rel32
            0x80..=0x8F => 4,
            // Various with Ib
            0x70..=0x73 | 0xA4 | 0xAC | 0xBA | 0xC2 | 0xC4..=0xC6 => 1,
            _ => 0,
        }
    } else if map == 3 {
        // 0F 3A - all have Ib
        1
    } else {
        0
    }
}

/// Read u16 little-endian
const fn read_u16_le(bytes: &[u8], pos: usize) -> u16 {
    (bytes[pos] as u16) | ((bytes[pos + 1] as u16) << 8)
}

/// Read u32 little-endian
const fn read_u32_le(bytes: &[u8], pos: usize) -> u32 {
    (bytes[pos] as u32)
        | ((bytes[pos + 1] as u32) << 8)
        | ((bytes[pos + 2] as u32) << 16)
        | ((bytes[pos + 3] as u32) << 24)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use crate::cpu::decoder::fetchdecode64::fetch_decode64;

    use super::*;

    #[test]
    fn test_nop() {
        // 0x90 is actually XCHG EAX,EAX which is the NOP encoding
        // In 64-bit mode with no REX.W, operand size is 32-bit
        let i = const_fetch_decode64(&[0x90]).unwrap();
        assert_eq!(i.ilen(), 1);
        // XchgErxEax is the 32-bit XCHG EAX,r32 - NOP is encoded as XCHG EAX,EAX
        assert_eq!(i.get_ia_opcode(), Opcode::XchgErxEax);
    }

    #[test]
    fn test_ret() {
        let i = const_fetch_decode64(&[0xC3]).unwrap();
        assert_eq!(i.ilen(), 1);
        assert_eq!(i.get_ia_opcode(), Opcode::RetOp64);
    }

    #[test]
    fn test_int3() {
        let i = const_fetch_decode64(&[0xCC]).unwrap();
        assert_eq!(i.ilen(), 1);
        assert_eq!(i.get_ia_opcode(), Opcode::INT3);
    }

    #[test]
    fn test_rex_w() {
        let i = const_fetch_decode64(&[0x48, 0x89, 0xC0]).unwrap(); // MOV RAX, RAX
        assert_eq!(i.ilen(), 3);
        assert!(i.os64_l() != 0);
    }

    #[test]
    fn test_modrm_reg() {
        let i = const_fetch_decode64(&[0x89, 0xD8]).unwrap(); // MOV EAX, EBX
        assert_eq!(i.ilen(), 2);
        assert!(i.mod_c0());
    }

    #[test]
    fn test_modrm_mem() {
        let i = const_fetch_decode64(&[0x8B, 0x03]).unwrap(); // MOV EAX, [RBX]
        assert_eq!(i.ilen(), 2);
        assert!(!i.mod_c0());
    }

    #[test]
    fn test_sib() {
        let i = const_fetch_decode64(&[0x8B, 0x04, 0x8B]).unwrap(); // MOV EAX, [RBX+RCX*4]
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.sib_scale(), 2); // *4
    }

    #[test]
    fn test_disp8() {
        let i = const_fetch_decode64(&[0x8B, 0x43, 0x10]).unwrap(); // MOV EAX, [RBX+0x10]
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.modrm_form.displacement.displ32u(), 0x10);
    }

    #[test]
    fn test_disp32() {
        let i = const_fetch_decode64(&[0x8B, 0x83, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.modrm_form.displacement.displ32u(), 0x12345678);
    }

    #[test]
    fn test_imm8() {
        let i = const_fetch_decode64(&[0x6A, 0x42]).unwrap(); // PUSH 0x42
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.modrm_form.operand_data.id(), 0x42);
    }

    #[test]
    fn test_imm32() {
        init_tracing();
        let i = const_fetch_decode64(&[0x68, 0x78, 0x56, 0x34, 0x12]).unwrap(); // PUSH 0x12345678
        tracing::info!("{i:#x?}");
        let i2 = fetch_decode64(&[0x68, 0x78, 0x56, 0x34, 0x12]); // PUSH 0x12345678
        tracing::info!("{i2:#x?}");
        assert_eq!(i.ilen(), 5);
        assert_eq!(i.modrm_form.operand_data.id(), 0x12345678);
    }

    #[test]
    fn test_0f_opcode() {
        let i = const_fetch_decode64(&[0x0F, 0xA2]).unwrap(); // CPUID
        assert_eq!(i.ilen(), 2);
    }

    #[test]
    fn test_jcc_rel32() {
        let i = const_fetch_decode64(&[0x0F, 0x84, 0x78, 0x56, 0x34, 0x12]).unwrap(); // JE
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.modrm_form.operand_data.id(), 0x12345678);
    }

    #[test]
    fn test_lock() {
        let i = const_fetch_decode64(&[0xF0, 0x87, 0x03]).unwrap(); // LOCK XCHG
        assert_eq!(i.ilen(), 3);
        // Check raw bits - expect bit 6 set for LOCK
        let bits = i.meta_info.metainfo1.bits();
        assert_eq!(
            i.lock_rep_used_value(),
            1,
            "lock_rep_used_value wrong, bits={}",
            bits
        );
        assert!(i.get_lock());
    }

    #[test]
    fn test_rep() {
        let i = const_fetch_decode64(&[0xF3, 0xA4]).unwrap(); // REP MOVSB
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.lock_rep_used_value(), 3);
    }

    #[test]
    fn test_empty() {
        let result = const_fetch_decode64(&[]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DecodeError::BufferUnderflow));
    }

    #[test]
    fn test_0f38() {
        let i = const_fetch_decode64(&[0x66, 0x0F, 0x38, 0x00, 0xC1]).unwrap(); // PSHUFB
        assert_eq!(i.ilen(), 5);
    }

    #[test]
    fn test_0f3a() {
        let i = const_fetch_decode64(&[0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x05]).unwrap(); // PALIGNR
        assert_eq!(i.ilen(), 6);
    }

    #[test]
    fn test_zydis_example_64bit() {
        init_tracing();
        //
        // xor rdi, rdi
        // xor rsi, rsi
        // xor rdx, rdx
        // xor rax, rax
        // push rax
        // mov rbx, 0x68732F2F6E69622F
        // push rbx
        // mov rdi, rsp
        // mov al, 0x3B
        // syscall
        let data = [
            0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x48, 0x31, 0xc0, 0x50, 0x48,
            0xbb, 0x2f, 0x62, 0x69, 0x6e, 0x2f, 0x2f, 0x73, 0x68, 0x53, 0x48, 0x89, 0xe7, 0xb0,
            0x3b, 0x0f, 0x05,
        ];

        let runtime_address = 0x007FFFFFFF400000;
        let instructions = const_disassemble_sequence_64bit(&data, runtime_address);

        for (len, instruction) in instructions {
            tracing::info!(
                "{:?} {} {} {:#x?}",
                instruction.get_ia_opcode(),
                instruction.dst(),
                instruction.src(),
                instruction
            );
        }
    }

    fn const_disassemble_sequence_64bit(
        data: &[u8],
        runtime_address: u64,
    ) -> Vec<(u64, BxInstructionGenerated)> {
        let mut offset = 0;
        let mut current_address = runtime_address;
        let mut instructions = Vec::new();

        while offset < data.len() {
            let remaining = &data[offset..];

            let decoded = match const_fetch_decode64(remaining) {
                Ok(instr) => instr,
                Err(_) => break,
            };

            let length = decoded.meta_info.ilen as usize;

            if length == 0 || offset + length > data.len() {
                // Invalid instruction or out of bounds
                tracing::error!("Invalid instruction length at offset {}", offset);
                break;
            }

            instructions.push((current_address, decoded));
            offset += length;
            current_address += length as u64;
            if decoded.get_ia_opcode() == Opcode::IaError {
                tracing::error!("Decode error at offset {}", offset);
            }
        }

        instructions
    }

    fn init_tracing() {
        use tracing_subscriber::fmt;
        let _ = fmt()
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
    }
}
