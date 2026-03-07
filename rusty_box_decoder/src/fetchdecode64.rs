//! Const-compatible 64-bit instruction decoder
//!
//! This module provides a `const fn` instruction decoder for x86-64 mode
//! that returns `BxInstruction` - the same structure used by
//! the non-const decoder.

use crate::instr_generated::Instruction;

use super::error::{DecodeError, DecodeResult};
use super::fetchdecode_generated::BxDecodeError;
use super::ia_opcodes::Opcode;
use super::instr::MetaInfoFlags;
use super::instr_generated::{BxInstructionMetaInfo, DisplacementData, ModRmForm, OperandData};
use super::BxSegregs;

// Import opcode tables
use super::fetchdecode_opmap::*;
use super::fetchdecode_opmap_0f38::BxOpcodeTable0F38;
use super::fetchdecode_opmap_0f3a::BxOpcodeTable0F3A;
use super::fetchdecode_x87::BX3_DNOW_OPCODE;

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

// Register constants for clarity
const BX_NIL_REGISTER: u8 = 19;
const BX_64BIT_REG_RIP: u8 = 16; // BX_GENERAL_REGISTERS = 16, matching Bochs
const BX_NO_INDEX: u8 = 4;

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
        // Match C++ exactly: Bit32u(op) & 0xFFFFFF and Bit32u(op >> 24)
        let ignmsk = (entry & 0xFFFFFF) as u32;
        let opmsk = (entry >> 24) as u32;

        if (opmsk & ignmsk) == (decmask & ignmsk) {
            let opcode_raw = ((entry >> 48) & 0x7FFF) as u16;
            return Opcode::from_u16_const(opcode_raw);
        }

        // Check if this is the last opcode (sign bit set) - matches C++ do-while condition
        // C++: while(Bit64s(op) > 0) means continue while sign bit is NOT set
        // So we break when sign bit IS set (entry < 0)
        if (entry as i64) < 0 {
            break;
        }

        i += 1;
    }
    Opcode::IaError
}

/// Const-compatible 64-bit instruction decoder
///
/// Decodes an x86-64 instruction and returns a `Result` containing either
/// a `BxInstruction` struct on success, or a `DecodeError` on failure.
/// This is the const fn equivalent of `fetch_decode64`.
pub const fn fetch_decode64(bytes: &[u8]) -> DecodeResult<Instruction> {
    let mut instr = Instruction {
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
    // Per Bochs fetchDecode64: REX prefixes do NOT break the prefix loop.
    // A legacy prefix after REX resets rex_prefix to 0.
    // Only a non-prefix byte breaks out.
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

            // Address size override — only clear As64 (Bochs: clearAs64 only)
            // Default 64-bit: As32=1, As64=1. With 0x67: As32=1, As64=0 (32-bit addressing)
            // Clearing both would give asize()=0 (16-bit — invalid in 64-bit mode)
            0x67 => {
                rex_prefix = 0;
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
                sse_prefix = 3; // Bochs: (0xF2 & 3) ^ 1 = 3 = SSE_PREFIX_F2
            }

            // REP/REPE/REPZ (also SSE prefix)
            0xF3 => {
                rex_prefix = 0;
                metainfo1_bits = (metainfo1_bits & 0x3F) | (3 << 6);
                sse_prefix = 2; // Bochs: (0xF3 & 3) ^ 1 = 2 = SSE_PREFIX_F3
            }

            // REX prefixes (0x40-0x4F)
            // Per Bochs: does NOT break, continues prefix loop (goto fetch_b1)
            // A subsequent legacy prefix will reset rex_prefix to 0
            // Store full byte (not b & 0x0F) so bare REX 0x40 is still non-zero.
            // Bochs: rex_prefix = b; — ensures REX.none (0x40) enables Extend8bit.
            0x40..=0x4F => {
                rex_prefix = b;
            }

            _ => break,
        }
        pos += 1;
    }

    // Post-prefix REX processing (matches Bochs fetchDecode64:1476-1482)
    // Must happen AFTER prefix loop so REX.W overrides any prior 0x66 prefix
    if rex_prefix != 0 {
        // assertExtend8bit: REX prefix enables extended 8-bit registers (SPL, BPL, SIL, DIL)
        metainfo1_bits |= MetaInfoFlags::Extend8bit.bits();
        if (rex_prefix & 0x08) != 0 {
            // REX.W: assert BOTH Os64 AND Os32 (Bochs assertOs64 + assertOs32)
            metainfo1_bits |= MetaInfoFlags::Os64.bits();
            metainfo1_bits |= MetaInfoFlags::Os32.bits();
        }
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
            return Err(DecodeError::Decoder(
                BxDecodeError::BxIllegalVexXopOpcodeMap,
            )); // VEX not fully supported in const
        }
    }

    if b1 == 0x62 {
        // EVEX prefix - simplified handling
        if pos + 2 < max_len && (bytes[pos] & 0x0C) == 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
            // EVEX not fully supported in const
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
            // 3DNow! (0F 0F) - use opcode_map = 4 to indicate 3DNow!
            // The suffix byte will be read AFTER ModRM and displacement
            opcode_map = 4;
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

    // REX.B extends register encoded in opcode low bits for non-ModRM opcodes
    // (PUSH r64, POP r64, MOV r64/imm, XCHG r64/RAX, BSWAP, INC/DEC)
    if !needs_modrm && (rex_prefix & 0x01) != 0 {
        rm |= 8;
    }

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
                    instr.meta_data[BX_INSTR_METADATA_BASE] = BX_NIL_REGISTER;
                }
            } else {
                instr.meta_data[BX_INSTR_METADATA_BASE] = (rm & 0xF) as u8;
                instr.meta_data[BX_INSTR_METADATA_INDEX] = BX_NO_INDEX;

                // Check for RIP-relative (mod=0, rm=5)
                if mod_field == 0 && (rm & 0x7) == 5 {
                    // RIP-relative addressing
                    if pos + 4 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u32_le(bytes, pos);
                    pos += 4;
                    instr.modrm_form.displacement.data32 = disp;
                    instr.meta_data[BX_INSTR_METADATA_BASE] = BX_64BIT_REG_RIP;
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
        // No ModRM - instruction uses register encoded in opcode (low 3 bits = rm)
        metainfo1_bits |= MetaInfoFlags::ModC0.bits();
    }

    // Store register fields
    // For ModRM instructions: DST=nnn (reg field), SRC1=rm (r/m field)
    // For non-ModRM instructions: depends on opcode encoding:
    //   - Most opcodes (B0-BF, 50-5F, 40-4F, 90-97): register in bits 0-2 (rm)
    //   - Segment push/pop (06,07,0E,16,17,1E,1F): segment in bits 3-5 (nnn)
    // Bochs uses assign_srcs() with source types (BX_SRC_NNN, BX_SRC_RM) to determine this
    if needs_modrm {
        // Validate segment register for MOV Ew,Sw (0x8C) and MOV Sw,Ew (0x8E)
        // Valid segment registers: ES(0), CS(1), SS(2), DS(3), FS(4), GS(5)
        // Invalid indices (6-7) should cause #UD per x86 specification
        if matches!(b1, 0x8C | 0x8E) && nnn > 5 {
            return Err(DecodeError::InvalidSegmentRegister {
                index: nnn as u8,
                opcode: b1 as u8,
            });
        }

        // Group opcodes: 80, 81, 83, C0, C1, D0-D3, F6, F7, FE, FF
        // For these, nnn field is the opcode extension (which operation), rm is the operand
        let is_group_opcode = matches!(
            b1,
            0x80 | 0x81
                | 0x83
                | 0xC0
                | 0xC1
                | 0xD0
                | 0xD1
                | 0xD2
                | 0xD3
                | 0xF6
                | 0xF7
                | 0xFE
                | 0xFF
        );

        // Segment register move instructions: 8C (MOV Ew,Sw) and 8E (MOV Sw,Ew)
        // For 0x8C: nnn=segment (source), rm=gpr (destination) -> DST=rm, SRC1=nnn
        // For 0x8E: nnn=segment (dest), rm=gpr (source) -> DST=nnn, SRC1=rm

        if is_group_opcode {
            // Group opcodes: operand is in rm, opcode extension in nnn
            instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
        } else if b1 == 0x8C {
            // MOV Ew,Sw: rm is destination (gpr), nnn is source (segment)
            instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
        } else if b1 == 0x8E {
            // MOV Sw,Ew: nnn is destination (segment), rm is source (gpr)
            instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;
        } else if (b1 < 0x100 && ((b1 & 0x0F) == 0x01 || (b1 & 0x0F) == 0x09))
            || b1 == 0x89
            // Two-byte Ed,Gd opcodes (DST=rm): Group 7, store-form SSE, MOV Rd/DRn, Groups 12-14
            || matches!(b1, 0x101 | 0x111 | 0x121 | 0x129 | 0x171 | 0x172 | 0x173)
            // BT/BTS/BTR/BTC EdGd (0F A3/AB/B3/BB): rm=bit-field(dst), nnn=bit-index(src)
            || matches!(b1, 0x1A3 | 0x1AB | 0x1B3 | 0x1BB)
            // XADD EbGb (0F C0), XADD EdGd (0F C1): rm=dst, nnn=src
            // CMPXCHG EbGb (0F B0), CMPXCHG EdGd (0F B1): rm=dst, nnn=src
            // MOVNTI Ed,Gd (0F C3): rm=mem(dst), nnn=gpr(src)
            || matches!(b1, 0x1B0 | 0x1B1 | 0x1C0 | 0x1C1 | 0x1C3)
            // BT/BTS/BTR/BTC Ev,Ib (0F BA /4../7): rm=operand(dst), nnn=opcode-ext(src)
            || b1 == 0x1BA
            // SHLD Ed,Gd,Ib/CL (0F A4/A5), SHRD Ed,Gd,Ib/CL (0F AC/AD):
            // rm=Ed=destination (shifted), nnn=Gd=source (provides bits)
            || matches!(b1, 0x1A4 | 0x1A5 | 0x1AC | 0x1AD)
            // SETcc Eb (0F 90..9F): single-operand, rm=destination, nnn=opcode extension
            || (b1 >= 0x190 && b1 <= 0x19F)
        {
            // Ed,Gd format: rm (Ed) is destination, nnn (Gd) is source
            // Examples: ADD Ed,Gd | SUB Ed,Gd | MOV Ed,Gd | BTS EdGd | XADD EbGb
            instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
        } else {
            // Gd,Ed format (opcodes 0x03, 0x0B, 0x13, 0x1B, 0x23, 0x2B, 0x33, 0x8B):
            // nnn (Gd) is destination, rm (Ed) is source
            // Examples: ADD Gd,Ed | SUB Gd,Ed | MOV Gd,Ed
            instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;
        }
    } else {
        // Check if this is a segment push/pop opcode (uses nnn for segment)
        // 06=PUSH ES, 07=POP ES, 0E=PUSH CS, 16=PUSH SS, 17=POP SS, 1E=PUSH DS, 1F=POP DS
        // Also 0FA0=PUSH FS, 0FA1=POP FS, 0FA8=PUSH GS, 0FA9=POP GS (two-byte)
        // Note: In 64-bit mode, 06/07/0E/16/17/1E/1F are invalid, only 0FAx forms exist
        let is_segment_push_pop = matches!(b1, 0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F)
            || (opcode_map == 1 && matches!(b1 & 0xFF, 0xA0 | 0xA1 | 0xA8 | 0xA9));

        if is_segment_push_pop {
            // Segment is in bits 3-5 (nnn)
            instr.meta_data[BX_INSTR_METADATA_DST] = nnn as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = rm as u8;
        } else {
            // Most non-ModRM: register in bits 0-2 (rm)
            instr.meta_data[BX_INSTR_METADATA_DST] = rm as u8;
            instr.meta_data[BX_INSTR_METADATA_SRC1] = nnn as u8;
        }
    }

    // === Phase 3.5: Read 3DNow! suffix byte (comes after ModRM/displacement) ===
    let mut dnow_suffix: u8 = 0;
    if opcode_map == 4 {
        // 3DNow! instructions: suffix byte is read AFTER ModRM and displacement
        if pos >= max_len {
            return Err(DecodeError::ImmediateBufferUnderflow);
        }
        dnow_suffix = bytes[pos];
        pos += 1;
    }

    // === Phase 4: Parse immediate ===
    // Pass nnn to distinguish Group 3a/3b variants (TEST vs NOT/NEG/etc)
    let imm_size = get_immediate_size_64(b1, opcode_map, sse_prefix, metainfo1_bits, nnn);

    if imm_size > 0 {
        if pos + (imm_size as usize) > max_len {
            return Err(DecodeError::ImmediateBufferUnderflow);
        }

        match imm_size {
            1 => {
                let byte_val = bytes[pos];
                // Sign-extend byte immediates that are used as 32-bit values via id():
                // - Branch opcodes (0x70-0x7F, 0xE0-0xE3, 0xEB): relative displacements
                // - 0x83 (Group 1 EqsIb): sign-extended imm8 to operand-size per Intel spec
                // - 0x6A (PUSH imm8): sign-extended to operand size
                // - 0x6B (IMUL r,r/m,imm8): sign-extended to operand size
                let needs_sign_ext = opcode_map == 0
                    && matches!(b1 as u8, 0x70..=0x7F | 0xE0..=0xE3 | 0xEB | 0x83 | 0x6A | 0x6B);
                instr.modrm_form.operand_data.id = if needs_sign_ext {
                    byte_val as i8 as i32 as u32
                } else {
                    byte_val as u32
                };
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
    if opcode_map == 4 {
        // 3DNow! instruction: use suffix to look up opcode directly
        instr.meta_info.ia_opcode = BX3_DNOW_OPCODE[dnow_suffix as usize];
    } else {
        instr.meta_info.ia_opcode = lookup_opcode_64(b1, opcode_map, decmask, nnn);
    }

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
            0x60..=0x62 | 0x68 | 0x6A | 0x6C..=0x6F |
            0x70..=0x7F |
            0x90..=0x9F |
            0xA0..=0xAF |
            0xB0..=0xBF |
            0xC2 | 0xC3 | 0xC8 | 0xC9 | 0xCA | 0xCB | 0xCC..=0xCF |
            0xD4..=0xD7 |
            0xE0..=0xEF |
            0xF1 | 0xF4 | 0xF5 | 0xF8..=0xFD
        )
    } else if map == 1 {
        // 0F map
        let opcode = (b1 & 0xFF) as u8;
        !matches!(opcode,
            0x05..=0x09 | 0x0B | 0x0E |
            0x30..=0x37 |
            0x77 |
            0x80..=0x8F |
            0xA0..=0xA2 | 0xA8..=0xAA |
            0xC8..=0xCF |
            0xFF
        )
    } else {
        // 0F38, 0F3A maps always need ModRM
        true
    }
}

/// Get immediate size for opcode (64-bit mode)
const fn get_immediate_size_64(b1: u32, map: u8, _sse_prefix: u8, metainfo1: u8, nnn: u32) -> u8 {
    let os32 = (metainfo1 & MetaInfoFlags::Os32.bits()) != 0;
    let os64 = (metainfo1 & MetaInfoFlags::Os64.bits()) != 0;
    let as64 = (metainfo1 & MetaInfoFlags::As64.bits()) != 0;

    if map == 0 {
        let opcode = b1 as u8;
        match opcode {
            // moffs (direct memory offset) - depends on ADDRESS size, not operand size
            // A0 = MOV AL, [moffs8]
            // A1 = MOV AX/EAX/RAX, [moffs]
            // A2 = MOV [moffs8], AL
            // A3 = MOV [moffs], AX/EAX/RAX
            0xA0 | 0xA1 | 0xA2 | 0xA3 => {
                if as64 {
                    8 // 64-bit address = 8-byte offset
                } else {
                    4 // 32-bit address = 4-byte offset (16-bit not used in 64-bit mode)
                }
            }

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

            // Group 3a (F6): TEST (nnn=0,1) has Ib, others have no immediate
            // Based on Bochs cpu/decoder/fetchdecode64.cc (fetchImmediate)
            0xF6 => {
                // Mask to 3 bits: REX.R may extend nnn to 8+
                if (nnn & 7) == 0 || (nnn & 7) == 1 {
                    1 // TEST r/m8, imm8
                } else {
                    0 // NOT/NEG/MUL/IMUL/DIV/IDIV - no immediate
                }
            }

            // Group 3b (F7): TEST (nnn=0,1) has Iv, others have no immediate
            0xF7 => {
                if (nnn & 7) == 0 || (nnn & 7) == 1 {
                    if os64 {
                        4 // TEST r/m64, imm32 (sign-extended)
                    } else if os32 {
                        4 // TEST r/m32, imm32
                    } else {
                        2 // TEST r/m16, imm16
                    }
                } else {
                    0 // NOT/NEG/MUL/IMUL/DIV/IDIV - no immediate
                }
            }

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
    extern crate std;

    use super::*;

    #[test]
    fn test_nop() {
        // 0x90 is actually XCHG EAX,EAX which is the NOP encoding
        // In 64-bit mode with no REX.W, operand size is 32-bit
        let i = fetch_decode64(&[0x90]).unwrap();
        assert_eq!(i.ilen(), 1);
        // XchgErxEax is the 32-bit XCHG EAX,r32 - NOP is encoded as XCHG EAX,EAX
        assert_eq!(i.get_ia_opcode(), Opcode::XchgErxEax);
    }

    #[test]
    fn test_ret() {
        let i = fetch_decode64(&[0xC3]).unwrap();
        assert_eq!(i.ilen(), 1);
        assert_eq!(i.get_ia_opcode(), Opcode::RetOp64);
    }

    #[test]
    fn test_int3() {
        let i = fetch_decode64(&[0xCC]).unwrap();
        assert_eq!(i.ilen(), 1);
        assert_eq!(i.get_ia_opcode(), Opcode::INT3);
    }

    #[test]
    fn test_rex_w() {
        let i = fetch_decode64(&[0x48, 0x89, 0xC0]).unwrap(); // MOV RAX, RAX
        assert_eq!(i.ilen(), 3);
        assert!(i.os64_l() != 0);
    }

    #[test]
    fn test_modrm_reg() {
        let i = fetch_decode64(&[0x89, 0xD8]).unwrap(); // MOV EAX, EBX
        assert_eq!(i.ilen(), 2);
        assert!(i.mod_c0());
    }

    #[test]
    fn test_modrm_mem() {
        let i = fetch_decode64(&[0x8B, 0x03]).unwrap(); // MOV EAX, [RBX]
        assert_eq!(i.ilen(), 2);
        assert!(!i.mod_c0());
    }

    #[test]
    fn test_sib() {
        let i = fetch_decode64(&[0x8B, 0x04, 0x8B]).unwrap(); // MOV EAX, [RBX+RCX*4]
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.sib_scale(), 2); // *4
    }

    #[test]
    fn test_disp8() {
        let i = fetch_decode64(&[0x8B, 0x43, 0x10]).unwrap(); // MOV EAX, [RBX+0x10]
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.modrm_form.displacement.displ32u(), 0x10);
    }

    #[test]
    fn test_disp32() {
        let i = fetch_decode64(&[0x8B, 0x83, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.modrm_form.displacement.displ32u(), 0x12345678);
    }

    #[test]
    fn test_imm8() {
        let i = fetch_decode64(&[0x6A, 0x42]).unwrap(); // PUSH 0x42
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.modrm_form.operand_data.id(), 0x42);
    }

    #[test]
    fn test_imm32() {
        init_tracing();
        let i = fetch_decode64(&[0x68, 0x78, 0x56, 0x34, 0x12]).unwrap(); // PUSH 0x12345678
        tracing::info!("{i:#x?}");
        let i2 = fetch_decode64(&[0x68, 0x78, 0x56, 0x34, 0x12]); // PUSH 0x12345678
        tracing::info!("{i2:#x?}");
        assert_eq!(i.ilen(), 5);
        assert_eq!(i.modrm_form.operand_data.id(), 0x12345678);
    }

    #[test]
    fn test_0f_opcode() {
        let i = fetch_decode64(&[0x0F, 0xA2]).unwrap(); // CPUID
        assert_eq!(i.ilen(), 2);
    }

    #[test]
    fn test_jcc_rel32() {
        let i = fetch_decode64(&[0x0F, 0x84, 0x78, 0x56, 0x34, 0x12]).unwrap(); // JE
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.modrm_form.operand_data.id(), 0x12345678);
    }

    #[test]
    fn test_lock() {
        let i = fetch_decode64(&[0xF0, 0x87, 0x03]).unwrap(); // LOCK XCHG
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
        let i = fetch_decode64(&[0xF3, 0xA4]).unwrap(); // REP MOVSB
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.lock_rep_used_value(), 3);
    }

    #[test]
    fn test_empty() {
        let result = fetch_decode64(&[]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DecodeError::BufferUnderflow));
    }

    #[test]
    fn test_0f38() {
        let i = fetch_decode64(&[0x66, 0x0F, 0x38, 0x00, 0xC1]).unwrap(); // PSHUFB
        assert_eq!(i.ilen(), 5);
    }

    #[test]
    fn test_0f3a() {
        let i = fetch_decode64(&[0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x05]).unwrap(); // PALIGNR
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

        for (_, instruction) in instructions {
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
    ) -> alloc::vec::Vec<(u64, Instruction)> {
        let mut offset = 0;
        let mut current_address = runtime_address;
        let mut instructions = alloc::vec::Vec::new();

        while offset < data.len() {
            let remaining = &data[offset..];

            let decoded = match fetch_decode64(remaining) {
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

    /// Test that valid segment registers (0-5) decode successfully for MOV Ew,Sw and MOV Sw,Ew
    #[test]
    fn test_mov_segment_valid() {
        // Test opcodes 0x8C (MOV r/m16, Sreg) and 0x8E (MOV Sreg, r/m16) with nnn=0 through nnn=5
        for seg in 0..=5 {
            let modrm = 0xC0 | (seg << 3); // MOD=11, REG=seg, R/M=0 (AX/RAX)

            // 0x8C: MOV r/m16, Sreg
            let bytes = vec![0x8C, modrm];
            let result = fetch_decode64(&bytes);
            assert!(
                result.is_ok(),
                "Failed to decode MOV Ew,Sw with valid segment {} (0x8C {:#04x})",
                seg,
                modrm
            );
            let instr = result.unwrap();
            assert_eq!(instr.get_ia_opcode(), Opcode::MovEwSw);
            assert_eq!(instr.meta_data[1], seg); // Source segment register

            // 0x8E: MOV Sreg, r/m16
            let bytes = vec![0x8E, modrm];
            let result = fetch_decode64(&bytes);
            assert!(
                result.is_ok(),
                "Failed to decode MOV Sw,Ew with valid segment {} (0x8E {:#04x})",
                seg,
                modrm
            );
            let instr = result.unwrap();
            assert_eq!(instr.get_ia_opcode(), Opcode::MovSwEw);
            assert_eq!(instr.meta_data[0], seg); // Destination segment register
        }
    }

    /// Test that invalid segment registers (6-7) are rejected with InvalidSegmentRegister error
    #[test]
    fn test_mov_segment_invalid() {
        // Test opcodes 0x8C and 0x8E with nnn=6 and nnn=7
        for seg in 6..=7 {
            let modrm = 0xC0 | (seg << 3); // MOD=11, REG=seg, R/M=0

            // 0x8C: MOV r/m16, Sreg - should fail with InvalidSegmentRegister
            let bytes = vec![0x8C, modrm];
            let result = fetch_decode64(&bytes);
            assert!(
                matches!(result, Err(DecodeError::InvalidSegmentRegister { index, opcode: 0x8C }) if index == seg),
                "Should reject invalid segment register {} for opcode 0x8C, got: {:?}",
                seg,
                result
            );

            // 0x8E: MOV Sreg, r/m16 - should fail with InvalidSegmentRegister
            let bytes = vec![0x8E, modrm];
            let result = fetch_decode64(&bytes);
            assert!(
                matches!(result, Err(DecodeError::InvalidSegmentRegister { index, opcode: 0x8E }) if index == seg),
                "Should reject invalid segment register {} for opcode 0x8E, got: {:?}",
                seg,
                result
            );
        }
    }
}
