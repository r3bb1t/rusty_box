//! 64-bit instruction decoder (matching Bochs `fetchdecode64.cc`).
//!
//! Provides `fetch_decode64` — a `const fn` decoder for x86-64 long mode
//! that produces an [`Instruction`].

use crate::instruction::{Instruction, InstructionFlags};
use crate::error::{DecodeError, DecodeResult};
use crate::opcode::Opcode;
use crate::BxSegregs;

use super::tables::{BxDecodeError, SsePrefix, VEX_W_OFFSET, VEX_VL_128_256_OFFSET, MASK_K0_OFFSET};

// Import opcode tables
use super::opmap::*;
use super::opmap_0f38::BxOpcodeTable0F38;
use super::opmap_0f3a::BxOpcodeTable0F3A;
use super::x87::{
    BX3_DNOW_OPCODE, BX_OPCODE_INFO_FLOATING_POINT_D8, BX_OPCODE_INFO_FLOATING_POINT_D9,
    BX_OPCODE_INFO_FLOATING_POINT_DA, BX_OPCODE_INFO_FLOATING_POINT_DB,
    BX_OPCODE_INFO_FLOATING_POINT_DC, BX_OPCODE_INFO_FLOATING_POINT_DD,
    BX_OPCODE_INFO_FLOATING_POINT_DE, BX_OPCODE_INFO_FLOATING_POINT_DF,
};

// Backward-compatible alias
use InstructionFlags as MetaInfoFlags;

// Register constants for clarity
const BX_NIL_REGISTER: u8 = 19;
const BX_64BIT_REG_RIP: u8 = 16; // BX_GENERAL_REGISTERS = 16, matching Bochs
const BX_NO_INDEX: u8 = 4;

const DS: u8 = BxSegregs::Ds as u8;
const SS: u8 = BxSegregs::Ss as u8;

// Segment default tables for 64-bit mode (matching Bochs fetchdecode64.cc lines 45-81)
// Index by base register (0-15). RSP(4)→SS, RBP(5)→SS in mod!=0; only RSP(4)→SS in mod==0
const SREG_MOD0_BASE32_64: [u8; 16] = [
    DS, DS, DS, DS, SS, DS, DS, DS, // base 0-7
    DS, DS, DS, DS, DS, DS, DS, DS, // base 8-15
];
const SREG_MOD1OR2_BASE32_64: [u8; 16] = [
    DS, DS, DS, DS, SS, SS, DS, DS, // base 0-7
    DS, DS, DS, DS, DS, DS, DS, DS, // base 8-15
];

// Decoding mask bit offsets
use super::tables::{
    AS32_OFFSET, AS64_OFFSET, IS64_OFFSET, LOCK_PREFIX_OFFSET, MODC0_OFFSET, NNN_OFFSET,
    OS32_OFFSET, OS64_OFFSET, RRR_OFFSET, SRC_EQ_DST_OFFSET, SSE_PREFIX_OFFSET,
};

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

/// Decode one x86-64 instruction from `bytes`.
///
/// Returns an [`Instruction`] on success, or [`DecodeError`] on failure.
pub const fn fetch_decode64(bytes: &[u8]) -> DecodeResult<Instruction> {
    let mut instr = Instruction {
        opcode: Opcode::IaError,
        length: 0,
        flags: InstructionFlags::empty(),
        operands: crate::instruction::Operands {
            dst: 0, src1: 0, src2: 0, src3: 0,
            segment: 0, base: 0, index: 0, scale: 0,
        },
        immediate: 0,
        displacement: 0,
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
    let mut sse_prefix: u8 = SsePrefix::PrefixNone as u8;
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
                if sse_prefix == SsePrefix::PrefixNone as u8 {
                    sse_prefix = SsePrefix::Prefix66 as u8;
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
                sse_prefix = SsePrefix::PrefixF2 as u8;
            }

            // REP/REPE/REPZ (also SSE prefix)
            0xF3 => {
                rex_prefix = 0;
                metainfo1_bits = (metainfo1_bits & 0x3F) | (3 << 6);
                sse_prefix = SsePrefix::PrefixF3 as u8;
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
        instr.operands.segment = seg_override;
    } else {
        instr.operands.segment = BxSegregs::Ds as u8;
    }

    // === Phase 2: Parse opcode ===
    let mut b1 = bytes[pos] as u32;
    pos += 1;

    // Check for VEX/EVEX/XOP prefixes
    let mut vex_vvv: u8 = 0; // VEX.vvvv register (0 = unused, stored inverted in encoding)
    let mut is_vex: bool = false;
    let mut is_evex: bool = false;
    let mut opcode_map: u8 = 0; // 0=1-byte, 1=0F, 2=0F38, 3=0F3A
    let mut vex_l: u8 = 0; // 0=128-bit (XMM), 1=256-bit (YMM), 2=512-bit (ZMM)
    let mut vex_w: u8 = 0; // VEX.W / EVEX.W bit
    let mut evex_z: u8 = 0; // EVEX zeroing-masking
    let mut evex_b_flag: u8 = 0; // EVEX broadcast/RC/SAE
    let mut evex_aaa: u8 = 0; // EVEX opmask register

    if b1 == 0xC4 || b1 == 0xC5 {
        // VEX prefix — in 64-bit mode, C4/C5 are always VEX (never LES/LDS)
        // Bochs decoder_vex64 (fetchdecode64.cc:764-883)
        if sse_prefix != SsePrefix::PrefixNone as u8 || rex_prefix != 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxIllegalVexXopWithRexPrefix));
        }

        is_vex = true;
        let mut vex_opc_map: u8 = 1; // 2-byte VEX implies map=1 (0F)
        let mut rex_x: u8 = 0;
        let mut rex_b: u8 = 0;

        if pos >= max_len {
            return Err(DecodeError::OpcodeBufferUnderflow);
        }
        let vex_byte1 = bytes[pos];
        pos += 1;

        // VEX.R is inverted: bit 7=0 means REX.R=1
        let rex_r = ((vex_byte1 >> 4) & 0x8) ^ 0x8;

        if b1 == 0xC4 {
            // 3-byte VEX prefix: C4 [RXBmmmmm] [WvvvvLpp]
            rex_x = ((vex_byte1 >> 3) & 0x8) ^ 0x8;
            rex_b = ((vex_byte1 >> 2) & 0x8) ^ 0x8;
            vex_opc_map = vex_byte1 & 0x1F;

            if pos >= max_len {
                return Err(DecodeError::OpcodeBufferUnderflow);
            }
            let vex_byte2 = bytes[pos];
            pos += 1;

            if (vex_byte2 & 0x80) != 0 {
                vex_w = 1;
                // VEX.W=1 implies 64-bit operand size
                metainfo1_bits |= MetaInfoFlags::Os64.bits() | MetaInfoFlags::Os32.bits();
            }

            vex_vvv = (15 - ((vex_byte2 >> 3) & 0xF)) as u8;
            vex_l = (vex_byte2 >> 2) & 0x1;
            sse_prefix = vex_byte2 & 0x3; // pp field = SSE prefix
        } else {
            // 2-byte VEX prefix: C5 [RvvvvLpp]
            vex_vvv = (15 - ((vex_byte1 >> 3) & 0xF)) as u8;
            vex_l = (vex_byte1 >> 2) & 0x1;
            sse_prefix = vex_byte1 & 0x3; // pp field = SSE prefix
        }

        // Build rex_prefix from VEX R/X/B bits (matching Bochs convention)
        // rex_prefix bit layout: 0=B, 1=X, 2=R, 3=W
        rex_prefix = (rex_b >> 3) | ((rex_x >> 3) << 1) | ((rex_r >> 3) << 2);

        // Read opcode byte
        if pos >= max_len {
            return Err(DecodeError::OpcodeBufferUnderflow);
        }
        let opcode_byte = bytes[pos] as u32;
        pos += 1;

        // Valid VEX maps: 1 (0F), 2 (0F38), 3 (0F3A)
        // Bochs fetchdecode64.cc: maps 0, 4, 5, 6 are invalid.
        // Map 7 exists in Bochs but has its OWN 256-entry table section
        // (indices 768-1023 in BxOpcodeTableVEX). We don't have those
        // entries, so accepting map 7 would decode wrong instructions.
        match vex_opc_map {
            1 => {
                b1 = 0x100 | opcode_byte;
                opcode_map = 1;
            }
            2 => {
                b1 = 0x200 | opcode_byte;
                opcode_map = 2;
            }
            3 => {
                b1 = 0x300 | opcode_byte;
                opcode_map = 3;
            }
            _ => {
                return Err(DecodeError::Decoder(BxDecodeError::BxIllegalVexXopOpcodeMap));
            }
        }

        // VZEROUPPER/VZEROALL (VEX.0F 77) has no ModRM
        // All other VEX instructions have ModRM
    }

    if b1 == 0x62 {
        // In 64-bit mode, 0x62 is always EVEX (never BOUND)
        // EVEX format: 62 [P0] [P1] [P2] [opcode] [modrm] ...
        // P0: ~R ~X ~B ~R' 00 mm
        // P1: W ~vvvv 1 pp
        // P2: z L'L b ~V' aaa
        if sse_prefix != SsePrefix::PrefixNone as u8 || rex_prefix != 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
        }
        if pos + 3 >= max_len {
            return Err(DecodeError::OpcodeBufferUnderflow);
        }

        is_vex = true; // treat EVEX like VEX for dispatch purposes
        is_evex = true;
        let p0 = bytes[pos];
        let p1 = bytes[pos + 1];
        let p2 = bytes[pos + 2];
        pos += 3;

        // P0: ~R(7) ~X(6) ~B(5) ~R'(4) 0(3) mmm(2:0)
        // Bochs: bit 3 must be 0 (reserved)
        if (p0 & 0x08) != 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
        }
        let evex_map = p0 & 0x07; // 3-bit map (Bochs: evex & 0x7)
        // R/X/B from P0 (inverted bits) — bit 3 extension for register encoding
        let rex_r_bit = if (p0 & 0x80) == 0 { 4u8 } else { 0u8 }; // ~R → REX.R (bit 2 of rex_prefix)
        let rex_x_bit = if (p0 & 0x40) == 0 { 2u8 } else { 0u8 }; // ~X → REX.X (bit 1)
        let rex_b_bit = if (p0 & 0x20) == 0 { 1u8 } else { 0u8 }; // ~B → REX.B (bit 0)
        // R' from P0 bit 4 — extends R to 5 bits for EVEX register encoding
        let _evex_r_prime = if (p0 & 0x10) == 0 { 1u8 } else { 0u8 }; // inverted

        // P1: W(7) ~vvvv(6:3) 1(2) pp(1:0)
        // Bit 2 must be 1
        if (p1 & 0x04) == 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
        }
        vex_w = (p1 >> 7) & 1;
        vex_vvv = (15 - ((p1 >> 3) & 0xF)) as u8;
        sse_prefix = p1 & 0x03;

        if vex_w != 0 {
            metainfo1_bits |= MetaInfoFlags::Os64.bits() | MetaInfoFlags::Os32.bits();
        }

        // P2: z(7) L'L(6:5) b(4) ~V'(3) aaa(2:0)
        vex_l = (p2 >> 5) & 0x03; // 0=128, 1=256, 2=512
        evex_z = (p2 >> 7) & 1;
        evex_b_flag = (p2 >> 4) & 1;
        evex_aaa = p2 & 0x07;
        // V' extends vvvv to 5 bits (inverted)
        let _evex_v_prime = if (p2 & 0x08) == 0 { 1u8 } else { 0u8 };

        rex_prefix = rex_b_bit | rex_x_bit | rex_r_bit;

        // Read opcode byte
        if pos >= max_len {
            return Err(DecodeError::OpcodeBufferUnderflow);
        }
        let opcode_byte = bytes[pos] as u32;
        pos += 1;

        // Bochs: map 0, 4, 7 are invalid; maps 5/6 valid but adjusted
        match evex_map {
            0 | 4 | 7 => {
                return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
            }
            1 => {
                b1 = 0x100 | opcode_byte;
                opcode_map = 1;
            }
            2 => {
                b1 = 0x200 | opcode_byte;
                opcode_map = 2;
            }
            3 => {
                b1 = 0x300 | opcode_byte;
                opcode_map = 3;
            }
            // Maps 5/6 (APX extensions) — adjust down by 1 to skip map 4
            5 => {
                b1 = 0x400 | opcode_byte;
                opcode_map = 4;
            }
            6 => {
                b1 = 0x500 | opcode_byte;
                opcode_map = 5;
            }
            _ => {
                return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
            }
        }

        // Validate z + k0: zeroing-masking with k0 is invalid (#UD) (Bochs fetchdecode64.cc:977-978)
        if evex_z != 0 && evex_aaa == 0 {
            return Err(DecodeError::Decoder(BxDecodeError::BxEvexReservedBitsSet));
        }
    }

    if b1 == 0x8F {
        // XOP prefix check — in 64-bit mode, bit 3 of next byte distinguishes XOP from POP
        // Bochs decoder_xop64: (*iptr & 0x08) != 0x08 → not XOP → decode as POP
        if pos < max_len && (bytes[pos] & 0x08) == 0x08 {
            return Err(DecodeError::Decoder(
                BxDecodeError::BxIllegalVexXopOpcodeMap,
            ));
        }
        // If bit 3 not set, fall through to decode as POP r/m64
    }

    // Two-byte escape (0F xx) — for non-VEX instructions
    if !is_vex && b1 == 0x0F {
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

    // === Phase 2.5: Check for UD64 opcodes (invalid in 64-bit mode) ===
    // Matching Bochs decoder_ud64 entries in decode64_descriptor table
    if opcode_map == 0 {
        let is_ud64 = matches!(
            b1 as u8,
            0x06 | 0x07       // PUSH/POP ES
            | 0x0E            // PUSH CS
            | 0x16 | 0x17     // PUSH/POP SS
            | 0x1E | 0x1F     // PUSH/POP DS
            | 0x27            // DAA
            | 0x2F            // DAS
            | 0x37            // AAA
            | 0x3F            // AAS
            | 0x60 | 0x61     // PUSHA/POPA
            | 0x82            // alias of Group 1 Eb,Ib
            | 0x9A            // CALL far ptr
            | 0xCE            // INTO
            | 0xD4 | 0xD5     // AAM/AAD
            | 0xD6            // SALC/SETALC
            | 0xEA            // JMP far ptr
        );
        if is_ud64 {
            return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
        }
    } else if opcode_map == 1 {
        // Two-byte UD64 opcodes (0F xx)
        let is_ud64_2byte = matches!(
            b1 & 0xFF,
            0x04 | 0x0A | 0x0C
            | 0x24 | 0x25 | 0x26 | 0x27
            | 0x36
            | 0x39
            | 0x3B | 0x3C | 0x3D | 0x3E | 0x3F
            | 0x7A | 0x7B
            | 0xA6 | 0xA7
        );
        if is_ud64_2byte {
            return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
        }
    }

    // === Phase 3: Parse ModRM if needed ===
    let needs_modrm = opcode_needs_modrm_64(b1, opcode_map);

    let mut nnn: u32 = (b1 >> 3) & 0x7;
    let mut rm: u32 = b1 & 0x7;
    let mut modrm_byte: u8 = 0; // full modrm byte, used for x87 FPU escape

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
        modrm_byte = modrm;
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

        // MOV CR/DR (0F 20-23) always treat as register form regardless of mod field
        // Matching Bochs decoder_creg64 which calls assertModC0()
        let force_modc0 = opcode_map == 1 && matches!(b1 & 0xFF, 0x20..=0x23);

        if mod_field == 3 || force_modc0 {
            // Register mode (or forced register for MOV CR/DR)
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

                instr.operands.scale = scale;
                instr.operands.index = index;
                instr.operands.base = base;

                // Displacement for SIB
                if mod_field == 0 && (base & 0x7) == 5 {
                    // [disp32] or [base+disp32]
                    if pos + 4 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u32_le(bytes, pos);
                    pos += 4;
                    instr.displacement = disp;
                    instr.operands.base = BX_NIL_REGISTER;
                }
            } else {
                instr.operands.base = (rm & 0xF) as u8;
                instr.operands.index = BX_NO_INDEX;

                // Check for RIP-relative (mod=0, rm=5)
                if mod_field == 0 && (rm & 0x7) == 5 {
                    // RIP-relative addressing
                    if pos + 4 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u32_le(bytes, pos);
                    pos += 4;
                    instr.displacement = disp;
                    instr.operands.base = BX_64BIT_REG_RIP;
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
                instr.displacement = disp;
            } else if mod_field == 2 {
                // disp32
                if pos + 4 > max_len {
                    return Err(DecodeError::DisplacementBufferUnderflow);
                }
                let disp = read_u32_le(bytes, pos);
                pos += 4;
                instr.displacement = disp;
            }

            // Apply segment default based on base register (Bochs sreg_mod0_base32 / sreg_mod1or2_base32)
            // Only when no explicit segment override was specified
            if seg_override >= 7 {
                let base_for_seg = instr.operands.base as usize;
                if base_for_seg < 16 {
                    let default_seg = if mod_field == 0 {
                        SREG_MOD0_BASE32_64[base_for_seg]
                    } else {
                        SREG_MOD1OR2_BASE32_64[base_for_seg]
                    };
                    instr.operands.segment = default_seg;
                }
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

        // Group opcodes: 80, 81, 83, C0, C1, C6, C7, D0-D3, F6, F7, FE, FF
        // For these, nnn field is the opcode extension (which operation), rm is the operand
        let is_group_opcode = matches!(
            b1,
            0x80 | 0x81
                | 0x83
                | 0xC0
                | 0xC1
                | 0xC6
                | 0xC7
                | 0xD0
                | 0xD1
                | 0xD2
                | 0xD3
                | 0xF6
                | 0xF7
                | 0xFE
                | 0xFF
                // Two-byte groups: dst=rm (operand), src1=nnn (opcode extension)
                // Matches Bochs convention where group opcodes always put rm in dst()
                | 0x100  // Group 6: SLDT/STR/LLDT/LTR/VERR/VERW (0F 00)
                | 0x1AE  // Group 15: FXSAVE/FXRSTOR/LDMXCSR/STMXCSR/CLFLUSH (0F AE)
                | 0x1C7  // Group 9: CMPXCHG8B/CMPXCHG16B (0F C7)
        );

        // Segment register move instructions: 8C (MOV Ew,Sw) and 8E (MOV Sw,Ew)
        // For 0x8C: nnn=segment (source), rm=gpr (destination) -> DST=rm, SRC1=nnn
        // For 0x8E: nnn=segment (dest), rm=gpr (source) -> DST=nnn, SRC1=rm

        if is_group_opcode {
            // Group opcodes: operand is in rm, opcode extension in nnn
            instr.operands.dst = rm as u8;
            instr.operands.src1 = nnn as u8;
        } else if b1 == 0x8C {
            // MOV Ew,Sw: rm is destination (gpr), nnn is source (segment)
            instr.operands.dst = rm as u8;
            instr.operands.src1 = nnn as u8;
        } else if b1 == 0x8E {
            // MOV Sw,Ew: nnn is destination (segment), rm is source (gpr)
            instr.operands.dst = nnn as u8;
            instr.operands.src1 = rm as u8;
        } else if (b1 < 0x100 && ((b1 & 0x0F) == 0x01 || (b1 & 0x0F) == 0x09) && b1 != 0x69)
            || b1 == 0x89
            // Two-byte Ed,Gd opcodes (DST=rm): Group 7, store-form SSE, MOV Rd/DRn, Groups 12-14
            || matches!(b1, 0x101 | 0x111 | 0x121 | 0x129 | 0x171 | 0x172 | 0x173)
            // SSE store-form opcodes: dst=rm(memory), src=nnn(xmm/mmx)
            // 0x17E (0F 7E): Ed,Gd for no-prefix (MOVD Ed,Pq) and 66 (MOVD Ed,Vd),
            // but NOT for F3 prefix (MOVQ Vq,Wq is a LOAD: nnn=dst, rm=src)
            || matches!(b1, 0x113 | 0x117 | 0x12B | 0x17F | 0x1E7)
            || (b1 == 0x17E && sse_prefix != SsePrefix::PrefixF3 as u8)
            // 0x1D6 (0F D6): Ed,Gd for 66 prefix (MOVQ Wq,Vq is a STORE: rm=dst, nnn=src),
            // but NOT for F2 (MOVDQ2Q) or F3 (MOVQ2DQ) which are LOADs (nnn=dst, rm=src)
            || (b1 == 0x1D6 && sse_prefix == SsePrefix::Prefix66 as u8)
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
            instr.operands.dst = rm as u8;
            instr.operands.src1 = nnn as u8;
        } else {
            // Gd,Ed format (opcodes 0x03, 0x0B, 0x13, 0x1B, 0x23, 0x2B, 0x33, 0x8B):
            // nnn (Gd) is destination, rm (Ed) is source
            // Examples: ADD Gd,Ed | SUB Gd,Ed | MOV Gd,Ed
            instr.operands.dst = nnn as u8;
            instr.operands.src1 = rm as u8;
        }
    } else {
        // Check if this is a segment push/pop opcode (uses nnn for segment)
        // 06=PUSH ES, 07=POP ES, 0E=PUSH CS, 16=PUSH SS, 17=POP SS, 1E=PUSH DS, 1F=POP DS
        // Also 0FA0=PUSH FS, 0FA1=POP FS, 0FA8=PUSH GS, 0FA9=POP GS (two-byte)
        // Note: In 64-bit mode, 06/07/0E/16/17/1E/1F are invalid, only 0FAx forms exist
        // Bochs convention: PUSH Sw has segment in src() (OP_NONE, OP_Sw),
        // POP Sw has segment in dst() (OP_Sw, OP_NONE)
        let is_segment_push = matches!(b1, 0x06 | 0x0E | 0x16 | 0x1E)
            || (opcode_map == 1 && matches!(b1 & 0xFF, 0xA0 | 0xA8));
        let is_segment_pop = matches!(b1, 0x07 | 0x17 | 0x1F)
            || (opcode_map == 1 && matches!(b1 & 0xFF, 0xA1 | 0xA9));

        if is_segment_push {
            // PUSH Sw: segment in src1 (Bochs: i->src())
            instr.operands.dst = rm as u8;
            instr.operands.src1 = nnn as u8;
        } else if is_segment_pop {
            // POP Sw: segment in dst (Bochs: i->dst())
            instr.operands.dst = nnn as u8;
            instr.operands.src1 = rm as u8;
        } else {
            // Most non-ModRM: register in bits 0-2 (rm)
            instr.operands.dst = rm as u8;
            instr.operands.src1 = nnn as u8;
        }
    }

    // Store VEX/EVEX fields in instruction
    if is_vex {
        instr.operands.src2 = vex_vvv;
        instr.set_vl(vex_l);
        instr.set_vex_w(vex_w);
        instr.set_vex(true);
        instr.flags = crate::instruction::InstructionFlags::from_bits_truncate(
            instr.flags.bits() | crate::instruction::InstructionFlags::VexPresent.bits()
        );
    }
    if is_evex {
        instr.set_opmask(evex_aaa);
        instr.set_evex_b(evex_b_flag);
        instr.set_zero_masking(evex_z);
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
                if needs_sign_ext {
                    // Sign-extended: overwrites full immediate (non-VEX branch/arith opcodes)
                    instr.immediate = byte_val as i8 as i32 as u32;
                } else {
                    // Write only byte 0, preserving bytes 1-3 (VL, VEX.W, VEX flags)
                    // This is critical for VEX instructions with imm8 (VPALIGNR, VPBLENDD, etc.)
                    let mut ib = instr.immediate.to_ne_bytes();
                    ib[0] = byte_val;
                    instr.immediate = u32::from_ne_bytes(ib);
                }
                pos += 1;
            }
            2 => {
                instr.immediate = read_u16_le(bytes, pos) as u32;
                pos += 2;
            }
            3 => {
                // ENTER: Iw (frame size) + Ib (nesting level)
                instr.immediate = read_u16_le(bytes, pos) as u32;
                instr.displacement = bytes[pos + 2] as u32;
                pos += 3;
            }
            4 => {
                instr.immediate = read_u32_le(bytes, pos);
                pos += 4;
            }
            8 => {
                // 64-bit immediate (MOV reg, imm64)
                // Store in displacement fields as we don't have iq field directly
                instr.immediate = read_u32_le(bytes, pos);
                instr.displacement = read_u32_le(bytes, pos + 4);
                pos += 8;
            }
            _ => {}
        }
    }

    // Finalize instruction
    instr.length = pos as u8;
    instr.flags = MetaInfoFlags::from_bits_retain(metainfo1_bits);

    // Build decmask for opcode lookup
    let mod_c0 = (metainfo1_bits & MetaInfoFlags::ModC0.bits()) != 0;
    let os64 = (metainfo1_bits & MetaInfoFlags::Os64.bits()) != 0;
    let os32 = (metainfo1_bits & MetaInfoFlags::Os32.bits()) != 0;
    let as64 = (metainfo1_bits & MetaInfoFlags::As64.bits()) != 0;
    let as32 = (metainfo1_bits & MetaInfoFlags::As32.bits()) != 0;

    // Bochs always includes nnn/rm in decmask, for both ModRM and non-ModRM opcodes.
    // For non-ModRM, nnn/rm come from opcode bits; for ModRM, from the ModRM byte.
    let lock_rep_value = (metainfo1_bits >> 6) & 0x3;
    let mut decmask: u32 = (if os64 { 1 } else { 0 } << OS64_OFFSET)
        | (if os32 { 1 } else { 0 } << OS32_OFFSET)
        | (if as64 { 1 } else { 0 } << AS64_OFFSET)
        | (if as32 { 1 } else { 0 } << AS32_OFFSET)
        | ((sse_prefix as u32) << SSE_PREFIX_OFFSET)
        | (if lock_rep_value == 1 { 1 } else { 0 } << LOCK_PREFIX_OFFSET)
        | (if mod_c0 { 1 } else { 0 } << MODC0_OFFSET)
        | (1 << IS64_OFFSET) // 64-bit mode
        | ((nnn & 0x7) << NNN_OFFSET)
        | ((rm & 0x7) << RRR_OFFSET)
        | ((vex_w as u32) << VEX_W_OFFSET)
        | ((vex_l as u32) << VEX_VL_128_256_OFFSET)
        | (if is_evex && evex_aaa == 0 { 1u32 << MASK_K0_OFFSET } else { 0 });
    // SRC_EQ_DST: Bochs sets this for zero-idiom detection (XOR reg,reg; SUB reg,reg)
    // Bochs uses full nnn == rm comparison (not masked to 3 bits) — prevents false positives
    // for register pairs like RAX/R8 where (nnn & 0x7) == (rm & 0x7) but nnn != rm
    if mod_c0 && nnn == rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }

    // Look up opcode from tables
    if opcode_map == 0 && (b1 >= 0xD8 && b1 <= 0xDF) {
        // x87 FPU escape opcodes — use dedicated FPU opcode tables
        // Matching Bochs decoder64_fp_escape() in fetchdecode64.cc
        let fpu_table = match b1 {
            0xD8 => &BX_OPCODE_INFO_FLOATING_POINT_D8,
            0xD9 => &BX_OPCODE_INFO_FLOATING_POINT_D9,
            0xDA => &BX_OPCODE_INFO_FLOATING_POINT_DA,
            0xDB => &BX_OPCODE_INFO_FLOATING_POINT_DB,
            0xDC => &BX_OPCODE_INFO_FLOATING_POINT_DC,
            0xDD => &BX_OPCODE_INFO_FLOATING_POINT_DD,
            0xDE => &BX_OPCODE_INFO_FLOATING_POINT_DE,
            _ => &BX_OPCODE_INFO_FLOATING_POINT_DF, // 0xDF
        };
        let fpu_index = if mod_c0 {
            // Register form: index = (modrm & 0x3F) + 8
            ((modrm_byte & 0x3F) as usize) + 8
        } else {
            // Memory form: index = nnn (0-7)
            (nnn & 0x7) as usize
        };
        instr.opcode = fpu_table[fpu_index];
        // Store foo: (modrm | (escape_byte << 8)) & 0x7FF — for x87 FPU handler context
        let foo_val = ((modrm_byte as u16) | ((b1 as u16) << 8)) & 0x7FF;
        instr.immediate = foo_val as u32;
    } else if opcode_map == 4 {
        // 3DNow! instruction: use suffix to look up opcode directly
        instr.opcode = BX3_DNOW_OPCODE[dnow_suffix as usize];
    } else if opcode_map == 0 && b1 == 0x90 {
        // Special NOP/PAUSE/XCHG handling (Bochs decoder64_nop)
        if (rex_prefix & 0x01) != 0 {
            // REX.B set: actual XCHG R8, RAX — use normal table
            instr.opcode = lookup_opcode_64(b1, opcode_map, decmask, nnn);
        } else if sse_prefix == SsePrefix::PrefixF3 as u8 {
            // F3 prefix → PAUSE
            instr.opcode = Opcode::Pause;
        } else {
            // Bare 0x90 → NOP
            instr.opcode = Opcode::Nop;
        }
    } else {
        instr.opcode = lookup_opcode_64(b1, opcode_map, decmask, nnn);
    }

    // EVEX opcode remapping: When EVEX prefix is present, try a direct EVEX
    // opcode lookup before falling back to the SSE/VEX tables. EVEX instructions
    // use distinct opcodes (e.g. VPXORD vs VPXOR) with per-element masking
    // granularity determined by EVEX.W. If the EVEX lookup succeeds, use it
    // directly — no SSE→VEX remapping needed.
    if is_evex {
        let w_bit = vex_w;
        if let Some(evex_op) = lookup_evex_opcode(opcode_map, (b1 & 0xFF) as u8, sse_prefix, w_bit) {
            instr.opcode = evex_op;
        }
    }

    // VEX SSE→VEX opcode remapping: When VEX prefix is present, the opcode table
    // may return an SSE opcode (e.g. PshufbVdqWdq) because SSE and VEX share the
    // same tables and SSE entries lack VEX attribute checks. SSE handlers are
    // 2-operand (dst=src1, ignore VEX.vvvv), but VEX instructions are 3-operand
    // (dst, vvv, rm). Remap to the proper VEX opcode so the 3-operand VEX handler
    // is dispatched. EVEX has its own tables and doesn't need this.
    if is_vex && !is_evex {
        instr.opcode = remap_sse_to_vex(instr.opcode, vex_l);
    }

    // Check if opcode lookup failed
    if matches!(instr.opcode, Opcode::IaError) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    // Post-decode LOCK validation (Bochs fetchdecode64.cc:1470-1478)
    // LOCK prefix on register operand (modC0) is always invalid → #UD
    let has_lock = (metainfo1_bits >> 6) & 0x3 == 1;
    if has_lock && mod_c0 {
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

/// Look up EVEX-specific opcode from the opcode map, opcode byte, SSE prefix, and W bit.
///
/// EVEX instructions have distinct opcodes from SSE/VEX (e.g. VPXORD/VPXORQ vs VPXOR)
/// because they support per-element masking with dword/qword granularity selected by EVEX.W.
/// This lookup is called before the normal SSE/VEX table so that EVEX-encoded instructions
/// get routed to the correct EVEX handlers in avx512.rs.
///
/// Returns `Some(opcode)` if a matching EVEX instruction is found, `None` otherwise.
///
/// `opcode_map`: 1=0F, 2=0F38, 3=0F3A
/// `opcode`: the opcode byte within the map
/// `sse_prefix`: 0=none, 1=66, 2=F3, 3=F2
/// `w`: EVEX.W bit (0 or 1)
const fn lookup_evex_opcode(opcode_map: u8, opcode: u8, sse_prefix: u8, w: u8) -> Option<Opcode> {
    match opcode_map {
        1 => {
            // Map 1 (0F xx)
            match (opcode, sse_prefix, w) {
                // VMOVDQU32/64 load — EVEX.F3.0F 6F
                (0x6F, 2, 0) => Some(Opcode::EvexVmovdqu32VdqWdq),
                (0x6F, 2, 1) => Some(Opcode::EvexVmovdqu64VdqWdq),
                // VMOVDQU32/64 store — EVEX.F3.0F 7F
                (0x7F, 2, 0) => Some(Opcode::EvexVmovdqu32WdqVdq),
                (0x7F, 2, 1) => Some(Opcode::EvexVmovdqu64WdqVdq),
                // VMOVDQA32/64 load — EVEX.66.0F 6F
                (0x6F, 1, 0) => Some(Opcode::EvexVmovdqa32VdqWdq),
                (0x6F, 1, 1) => Some(Opcode::EvexVmovdqa64VdqWdq),
                // VMOVDQA32/64 store — EVEX.66.0F 7F
                (0x7F, 1, 0) => Some(Opcode::EvexVmovdqa32WdqVdq),
                (0x7F, 1, 1) => Some(Opcode::EvexVmovdqa64WdqVdq),
                // VPADDD — EVEX.66.0F.W0 FE
                (0xFE, 1, 0) => Some(Opcode::EvexVpadddVdqHdqWdq),
                // VPADDQ — EVEX.66.0F.W1 D4
                (0xD4, 1, 1) => Some(Opcode::EvexVpaddqVdqHdqWdq),
                // VPSUBD — EVEX.66.0F.W0 FA
                (0xFA, 1, 0) => Some(Opcode::EvexVpsubdVdqHdqWdq),
                // VPSUBQ — EVEX.66.0F.W1 FB
                (0xFB, 1, 1) => Some(Opcode::EvexVpsubqVdqHdqWdq),
                // VPXORD — EVEX.66.0F.W0 EF
                (0xEF, 1, 0) => Some(Opcode::EvexVpxordVdqHdqWdq),
                // VPXORQ — EVEX.66.0F.W1 EF
                (0xEF, 1, 1) => Some(Opcode::EvexVpxorqVdqHdqWdq),
                // VPORD — EVEX.66.0F.W0 EB
                (0xEB, 1, 0) => Some(Opcode::EvexVpordVdqHdqWdq),
                // VPORQ — EVEX.66.0F.W1 EB
                (0xEB, 1, 1) => Some(Opcode::EvexVporqVdqHdqWdq),
                // VPANDD — EVEX.66.0F.W0 DB
                (0xDB, 1, 0) => Some(Opcode::EvexVpanddVdqHdqWdq),
                // VPANDQ — EVEX.66.0F.W1 DB
                (0xDB, 1, 1) => Some(Opcode::EvexVpandqVdqHdqWdq),
                // VPANDND — EVEX.66.0F.W0 DF
                (0xDF, 1, 0) => Some(Opcode::EvexVpandndVdqHdqWdq),
                // VPANDNQ — EVEX.66.0F.W1 DF
                (0xDF, 1, 1) => Some(Opcode::EvexVpandnqVdqHdqWdq),
                // VPSHUFD — EVEX.66.0F.W0 70
                (0x70, 1, 0) => Some(Opcode::EvexVpshufdVdqWdqIb),
                // VPCMPEQD → opmask — EVEX.66.0F.W0 76
                (0x76, 1, 0) => Some(Opcode::EvexVpcmpeqdKgwHdqWdq),
                // VPCMPGTD → opmask — EVEX.66.0F.W0 66
                (0x66, 1, 0) => Some(Opcode::EvexVpcmpgtdKgwHdqWdq),
                // VPMULUDQ — EVEX.66.0F.W1 F4
                (0xF4, 1, 1) => Some(Opcode::EvexVpmuludqVdqHdqWdq),
                // VMOVUPS load — EVEX.0F.W0 10 (no prefix)
                (0x10, 0, 0) => Some(Opcode::EvexVmovupsVpsWps),
                // VMOVUPD load — EVEX.66.0F.W1 10
                (0x10, 1, 1) => Some(Opcode::EvexVmovupdVpdWpd),
                // VMOVSS load — EVEX.F3.0F.W0 10
                (0x10, 2, 0) => Some(Opcode::EvexVmovssVssWss),
                // VMOVSD load — EVEX.F2.0F.W1 10
                (0x10, 3, 1) => Some(Opcode::EvexVmovsdVsdWsd),
                // VMOVUPS store — EVEX.0F.W0 11
                (0x11, 0, 0) => Some(Opcode::EvexVmovupsWpsVps),
                // VMOVUPD store — EVEX.66.0F.W1 11
                (0x11, 1, 1) => Some(Opcode::EvexVmovupdWpdVpd),
                // VMOVSS store — EVEX.F3.0F.W0 11
                (0x11, 2, 0) => Some(Opcode::EvexVmovssWssVss),
                // VMOVSD store — EVEX.F2.0F.W1 11
                (0x11, 3, 1) => Some(Opcode::EvexVmovsdWsdVsd),
                // VMOVAPS load — EVEX.0F.W0 28 (no prefix)
                (0x28, 0, 0) => Some(Opcode::EvexVmovapsVpsWps),
                // VMOVAPD load — EVEX.66.0F.W1 28
                (0x28, 1, 1) => Some(Opcode::EvexVmovapdVpdWpd),
                // VMOVAPS store — EVEX.0F.W0 29
                (0x29, 0, 0) => Some(Opcode::EvexVmovapsWpsVps),
                // VMOVAPD store — EVEX.66.0F.W1 29
                (0x29, 1, 1) => Some(Opcode::EvexVmovapdWpdVpd),
                // VPUNPCKLDQ — EVEX.66.0F.W0 62
                (0x62, 1, 0) => Some(Opcode::EvexVpunpckldqVdqHdqWdq),
                // VPUNPCKHDQ — EVEX.66.0F.W0 6A
                (0x6A, 1, 0) => Some(Opcode::EvexVpunpckhdqVdqHdqWdq),
                // VPUNPCKLQDQ — EVEX.66.0F.W1 6C
                (0x6C, 1, 1) => Some(Opcode::EvexVpunpcklqdqVdqHdqWdq),
                // VPUNPCKHQDQ — EVEX.66.0F.W1 6D
                (0x6D, 1, 1) => Some(Opcode::EvexVpunpckhqdqVdqHdqWdq),
                // Shift by XMM register
                (0xF2, 1, 0) => Some(Opcode::EvexVpslldVdqHdqWdq),  // VPSLLD
                (0xF3, 1, 1) => Some(Opcode::EvexVpsllqVdqHdqWdq),  // VPSLLQ
                (0xD2, 1, 0) => Some(Opcode::EvexVpsrldVdqHdqWdq),  // VPSRLD
                (0xD3, 1, 1) => Some(Opcode::EvexVpsrlqVdqHdqWdq),  // VPSRLQ
                (0xE2, 1, 0) => Some(Opcode::EvexVpsradVdqHdqWdq),  // VPSRAD
                (0xE2, 1, 1) => Some(Opcode::EvexVpsraqVdqHdqWdq),  // VPSRAQ

                // --- FP arithmetic (avx512.rs packed, avx512_scalar.rs scalar) ---
                // VADDPS/PD — EVEX.0F 58
                (0x58, 0, 0) => Some(Opcode::EvexVaddpsVpsHpsWps),
                (0x58, 1, 1) => Some(Opcode::EvexVaddpdVpdHpdWpd),
                (0x58, 2, 0) => Some(Opcode::EvexVaddssVssHpsWss),
                (0x58, 3, 1) => Some(Opcode::EvexVaddsdVsdHpdWsd),
                // VSUBPS/PD — EVEX.0F 5C
                (0x5C, 0, 0) => Some(Opcode::EvexVsubpsVpsHpsWps),
                (0x5C, 1, 1) => Some(Opcode::EvexVsubpdVpdHpdWpd),
                (0x5C, 2, 0) => Some(Opcode::EvexVsubssVssHpsWss),
                (0x5C, 3, 1) => Some(Opcode::EvexVsubsdVsdHpdWsd),
                // VMULPS/PD — EVEX.0F 59
                (0x59, 0, 0) => Some(Opcode::EvexVmulpsVpsHpsWps),
                (0x59, 1, 1) => Some(Opcode::EvexVmulpdVpdHpdWpd),
                (0x59, 2, 0) => Some(Opcode::EvexVmulssVssHpsWss),
                (0x59, 3, 1) => Some(Opcode::EvexVmulsdVsdHpdWsd),
                // VDIVPS/PD — EVEX.0F 5E
                (0x5E, 0, 0) => Some(Opcode::EvexVdivpsVpsHpsWps),
                (0x5E, 1, 1) => Some(Opcode::EvexVdivpdVpdHpdWpd),
                (0x5E, 2, 0) => Some(Opcode::EvexVdivssVssHpsWss),
                (0x5E, 3, 1) => Some(Opcode::EvexVdivsdVsdHpdWsd),
                // VMINPS/PD — EVEX.0F 5D
                (0x5D, 0, 0) => Some(Opcode::EvexVminpsVpsHpsWps),
                (0x5D, 1, 1) => Some(Opcode::EvexVminpdVpdHpdWpd),
                (0x5D, 2, 0) => Some(Opcode::EvexVminssVssHpsWss),
                (0x5D, 3, 1) => Some(Opcode::EvexVminsdVsdHpdWsd),
                // VMAXPS/PD — EVEX.0F 5F
                (0x5F, 0, 0) => Some(Opcode::EvexVmaxpsVpsHpsWps),
                (0x5F, 1, 1) => Some(Opcode::EvexVmaxpdVpdHpdWpd),
                (0x5F, 2, 0) => Some(Opcode::EvexVmaxssVssHpsWss),
                (0x5F, 3, 1) => Some(Opcode::EvexVmaxsdVsdHpdWsd),
                // VSQRTPS/PD — EVEX.0F 51
                (0x51, 0, 0) => Some(Opcode::EvexVsqrtpsVpsWps),
                (0x51, 1, 1) => Some(Opcode::EvexVsqrtpdVpdWpd),
                (0x51, 2, 0) => Some(Opcode::EvexVsqrtssVssHpsWss),
                (0x51, 3, 1) => Some(Opcode::EvexVsqrtsdVsdHpdWsd),

                // --- FP conversions (avx512_cvt.rs) ---
                // VCVTDQ2PS — EVEX.0F.W0 5B (no prefix)
                (0x5B, 0, 0) => Some(Opcode::EvexVcvtdq2psVpsWdq),
                // VCVTPS2DQ — EVEX.66.0F.W0 5B
                (0x5B, 1, 0) => Some(Opcode::EvexVcvtps2dqVdqWps),
                // VCVTTPS2DQ — EVEX.F3.0F.W0 5B
                (0x5B, 2, 0) => Some(Opcode::EvexVcvttps2dqVdqWps),
                // VCVTPS2PD — EVEX.0F.W0 5A (no prefix)
                (0x5A, 0, 0) => Some(Opcode::EvexVcvtps2pdVpdWps),
                // VCVTPD2PS — EVEX.66.0F.W1 5A
                (0x5A, 1, 1) => Some(Opcode::EvexVcvtpd2psVpsWpd),
                // VCVTDQ2PD — EVEX.F3.0F.W0 E6
                (0xE6, 2, 0) => Some(Opcode::EvexVcvtdq2pdVpdWdq),
                // VCVTPD2DQ — EVEX.F2.0F.W1 E6
                (0xE6, 3, 1) => Some(Opcode::EvexVcvtpd2dqVdqWpd),
                // VCVTTPD2DQ — EVEX.66.0F.W1 E6
                (0xE6, 1, 1) => Some(Opcode::EvexVcvttpd2dqVdqWpd),
                // VCVTUDQ2PS — EVEX.F2.0F.W0 7A
                (0x7A, 3, 0) => Some(Opcode::EvexVcvtudq2psVpsWdq),
                // VCVTPS2UDQ — EVEX.0F.W0 79
                (0x79, 0, 0) => Some(Opcode::EvexVcvtps2udqVdqWps),
                // VCVTTPS2UDQ — EVEX.0F.W0 78
                (0x78, 0, 0) => Some(Opcode::EvexVcvttps2udqVdqWps),

                // --- FP compare (avx512_cmp.rs) ---
                // VCMPPS — EVEX.0F.W0 C2 (no prefix)
                (0xC2, 0, 0) => Some(Opcode::EvexVcmppsKgwHpsWpsIb),
                // VCMPPD — EVEX.66.0F.W1 C2
                (0xC2, 1, 1) => Some(Opcode::EvexVcmppdKgbHpdWpdIb),

                // --- FP shuffle/unpack (avx512_perm.rs) ---
                // VUNPCKLPS — EVEX.0F.W0 14
                (0x14, 0, 0) => Some(Opcode::EvexVunpcklpsVpsHpsWps),
                // VUNPCKHPS — EVEX.0F.W0 15
                (0x15, 0, 0) => Some(Opcode::EvexVunpckhpsVpsHpsWps),
                // VUNPCKLPD — EVEX.66.0F.W1 14
                (0x14, 1, 1) => Some(Opcode::EvexVunpcklpdVpdHpdWpd),
                // VUNPCKHPD — EVEX.66.0F.W1 15
                (0x15, 1, 1) => Some(Opcode::EvexVunpckhpdVpdHpdWpd),
                // VSHUFPS — EVEX.0F.W0 C6 (no prefix)
                (0xC6, 0, 0) => Some(Opcode::EvexVshufpsVpsHpsWpsIb),
                // VSHUFPD — EVEX.66.0F.W1 C6
                (0xC6, 1, 1) => Some(Opcode::EvexVshufpdVpdHpdWpdIb),

                // --- BW byte/word ops (avx512_bw.rs) ---
                // VPADDB — EVEX.66.0F.W0 FC
                (0xFC, 1, 0) => Some(Opcode::EvexVpaddbVdqHdqWdq),
                // VPADDW — EVEX.66.0F.W0 FD
                (0xFD, 1, 0) => Some(Opcode::EvexVpaddwVdqHdqWdq),
                // VPSUBB — EVEX.66.0F.W0 F8
                (0xF8, 1, 0) => Some(Opcode::EvexVpsubbVdqHdqWdq),
                // VPSUBW — EVEX.66.0F.W0 F9
                (0xF9, 1, 0) => Some(Opcode::EvexVpsubwVdqHdqWdq),
                // VPMULLW — EVEX.66.0F.W0 D5
                (0xD5, 1, 0) => Some(Opcode::EvexVpmullwVdqHdqWdq),
                // VPAVGB — EVEX.66.0F.W0 E0
                (0xE0, 1, 0) => Some(Opcode::EvexVpavgbVdqHdqWdq),
                // VPAVGW — EVEX.66.0F.W0 E3
                (0xE3, 1, 0) => Some(Opcode::EvexVpavgwVdqHdqWdq),
                // VPMAXUB — EVEX.66.0F.W0 DE
                (0xDE, 1, 0) => Some(Opcode::EvexVpmaxubVdqHdqWdq),
                // VPMINUB — EVEX.66.0F.W0 DA
                (0xDA, 1, 0) => Some(Opcode::EvexVpminubVdqHdqWdq),
                // VPMAXSW — EVEX.66.0F.W0 EE
                (0xEE, 1, 0) => Some(Opcode::EvexVpmaxswVdqHdqWdq),
                // VPMINSW — EVEX.66.0F.W0 EA
                (0xEA, 1, 0) => Some(Opcode::EvexVpminswVdqHdqWdq),
                // VPACKSSDW — EVEX.66.0F.W0 6B
                (0x6B, 1, 0) => Some(Opcode::EvexVpackssdwVdqHdqWdq),
                // VPUNPCKLBW — EVEX.66.0F.W0 60
                (0x60, 1, 0) => Some(Opcode::EvexVpunpcklbwVdqHdqWdq),
                // VPUNPCKHBW — EVEX.66.0F.W0 68
                (0x68, 1, 0) => Some(Opcode::EvexVpunpckhbwVdqHdqWdq),
                // VPUNPCKLWD — EVEX.66.0F.W0 61
                (0x61, 1, 0) => Some(Opcode::EvexVpunpcklwdVdqHdqWdq),
                // VPUNPCKHWD — EVEX.66.0F.W0 69
                (0x69, 1, 0) => Some(Opcode::EvexVpunpckhwdVdqHdqWdq),

                // --- Integer (avx512_int.rs) in Map 1 ---
                // VPMULHUW — EVEX.66.0F.W0 E4
                (0xE4, 1, 0) => Some(Opcode::EvexVpmulhuwVdqHdqWdq),
                // VPMULHW — EVEX.66.0F.W0 E5
                (0xE5, 1, 0) => Some(Opcode::EvexVpmulhwVdqHdqWdq),
                // VPMADDWD — EVEX.66.0F.W0 F5
                (0xF5, 1, 0) => Some(Opcode::EvexVpmaddwdVdqHdqWdq),
                // VPSADBW — EVEX.66.0F.W0 F6
                (0xF6, 1, 0) => Some(Opcode::EvexVpsadbwVdqHdqWdq),

                // --- FP logical (Map 1) ---
                // VANDPS — EVEX.0F.W0 54
                (0x54, 0, 0) => Some(Opcode::EvexVandpsVpsHpsWps),
                // VANDPD — EVEX.66.0F.W1 54
                (0x54, 1, 1) => Some(Opcode::EvexVandpdVpdHpdWpd),
                // VANDNPS — EVEX.0F.W0 55
                (0x55, 0, 0) => Some(Opcode::EvexVandnpsVpsHpsWps),
                // VANDNPD — EVEX.66.0F.W1 55
                (0x55, 1, 1) => Some(Opcode::EvexVandnpdVpdHpdWpd),
                // VORPS — EVEX.0F.W0 56
                (0x56, 0, 0) => Some(Opcode::EvexVorpsVpsHpsWps),
                // VORPD — EVEX.66.0F.W1 56
                (0x56, 1, 1) => Some(Opcode::EvexVorpdVpdHpdWpd),
                // VXORPS — EVEX.0F.W0 57
                (0x57, 0, 0) => Some(Opcode::EvexVxorpsVpsHpsWps),
                // VXORPD — EVEX.66.0F.W1 57
                (0x57, 1, 1) => Some(Opcode::EvexVxorpdVpdHpdWpd),

                _ => None,
            }
        }
        2 => {
            // Map 2 (0F 38 xx)
            match (opcode, sse_prefix, w) {
                // VPBROADCASTD — EVEX.66.0F38.W0 58
                (0x58, 1, 0) => Some(Opcode::EvexVpbroadcastdVdqWd),
                // VPBROADCASTQ — EVEX.66.0F38.W1 59
                (0x59, 1, 1) => Some(Opcode::EvexVpbroadcastqVdqWq),
                // VPBROADCASTD from GPR — EVEX.66.0F38.W0 7C
                (0x7C, 1, 0) => Some(Opcode::EvexVpbroadcastdVdqEd),
                // VPBROADCASTQ from GPR — EVEX.66.0F38.W1 7C
                (0x7C, 1, 1) => Some(Opcode::EvexVpbroadcastqVdqEq),
                // VPSHUFB — EVEX.66.0F38.W0 00
                (0x00, 1, 0) => Some(Opcode::EvexVpshufbVdqHdqWdq),
                // VPMULLD — EVEX.66.0F38.W0 40
                (0x40, 1, 0) => Some(Opcode::EvexVpmulldVdqHdqWdq),
                // VPMINSD — EVEX.66.0F38.W0 39
                (0x39, 1, 0) => Some(Opcode::EvexVpminsdVdqHdqWdq),
                // VPABSD — EVEX.66.0F38.W0 1E
                (0x1E, 1, 0) => Some(Opcode::EvexVpabsdVdqWdq),
                // VPABSQ — EVEX.66.0F38.W1 1F
                (0x1F, 1, 1) => Some(Opcode::EvexVpabsqVdqWdq),
                // VPMAXSD — EVEX.66.0F38.W0 3D
                (0x3D, 1, 0) => Some(Opcode::EvexVpmaxsdVdqHdqWdq),
                // VPMAXSQ — EVEX.66.0F38.W1 3D
                (0x3D, 1, 1) => Some(Opcode::EvexVpmaxsqVdqHdqWdq),
                // VPMINSQ — EVEX.66.0F38.W1 39
                (0x39, 1, 1) => Some(Opcode::EvexVpminsqVdqHdqWdq),
                // Variable rotates
                (0x14, 1, 0) => Some(Opcode::EvexVprorvdVdqHdqWdq),
                (0x14, 1, 1) => Some(Opcode::EvexVprorvqVdqHdqWdq),
                (0x15, 1, 0) => Some(Opcode::EvexVprolvdVdqHdqWdq),
                (0x15, 1, 1) => Some(Opcode::EvexVprolvqVdqHdqWdq),
                // Sign/zero extend dword→qword
                (0x25, 1, 0) => Some(Opcode::EvexVpmovsxdqVdqWdq),
                (0x35, 1, 0) => Some(Opcode::EvexVpmovzxdqVdqWdq),
                // VPCMPEQQ — EVEX.66.0F38.W1 29
                (0x29, 1, 1) => Some(Opcode::EvexVpcmpeqqKgbHdqWdq),
                // VPCMPGTQ — EVEX.66.0F38.W1 37
                (0x37, 1, 1) => Some(Opcode::EvexVpcmpgtqKgbHdqWdq),
                // Variable shifts
                (0x45, 1, 0) => Some(Opcode::EvexVpsrlvdVdqHdqWdq),
                (0x45, 1, 1) => Some(Opcode::EvexVpsrlvqVdqHdqWdq),
                (0x46, 1, 0) => Some(Opcode::EvexVpsravdVdqHdqWdq),
                (0x46, 1, 1) => Some(Opcode::EvexVpsravqVdqHdqWdq),
                (0x47, 1, 0) => Some(Opcode::EvexVpsllvdVdqHdqWdq),
                (0x47, 1, 1) => Some(Opcode::EvexVpsllvqVdqHdqWdq),
                // VPBLENDMD — EVEX.66.0F38.W0 64
                (0x64, 1, 0) => Some(Opcode::EvexVpblendmdVdqHdqWdq),
                // VPBLENDMQ — EVEX.66.0F38.W1 64
                (0x64, 1, 1) => Some(Opcode::EvexVpblendmqVdqHdqWdq),
                // VPERMD — EVEX.66.0F38.W0 36
                (0x36, 1, 0) => Some(Opcode::EvexVpermdVdqHdqWdqKmask),
                // VPERMQ — EVEX.66.0F38.W1 36
                (0x36, 1, 1) => Some(Opcode::EvexVpermqVdqHdqWdqKmask),

                // --- FMA (avx512_fma.rs) ---
                // VFMADD132PS/PD
                (0x98, 1, 0) => Some(Opcode::EvexVfmadd132psVpsHpsWps),
                (0x98, 1, 1) => Some(Opcode::EvexVfmadd132pdVpdHpdWpd),
                // VFMADD213PS/PD
                (0xA8, 1, 0) => Some(Opcode::EvexVfmadd213psVpsHpsWps),
                (0xA8, 1, 1) => Some(Opcode::EvexVfmadd213pdVpdHpdWpd),
                // VFMADD231PS/PD
                (0xB8, 1, 0) => Some(Opcode::EvexVfmadd231psVpsHpsWps),
                (0xB8, 1, 1) => Some(Opcode::EvexVfmadd231pdVpdHpdWpd),
                // VFMSUB132PS/PD
                (0x9A, 1, 0) => Some(Opcode::EvexVfmsub132psVpsHpsWps),
                (0x9A, 1, 1) => Some(Opcode::EvexVfmsub132pdVpdHpdWpd),
                // VFMSUB213PS/PD
                (0xAA, 1, 0) => Some(Opcode::EvexVfmsub213psVpsHpsWps),
                (0xAA, 1, 1) => Some(Opcode::EvexVfmsub213pdVpdHpdWpd),
                // VFMSUB231PS/PD
                (0xBA, 1, 0) => Some(Opcode::EvexVfmsub231psVpsHpsWps),
                (0xBA, 1, 1) => Some(Opcode::EvexVfmsub231pdVpdHpdWpd),
                // VFNMADD132PS/PD
                (0x9C, 1, 0) => Some(Opcode::EvexVfnmadd132psVpsHpsWps),
                (0x9C, 1, 1) => Some(Opcode::EvexVfnmadd132pdVpdHpdWpd),
                // VFNMADD213PS/PD
                (0xAC, 1, 0) => Some(Opcode::EvexVfnmadd213psVpsHpsWps),
                (0xAC, 1, 1) => Some(Opcode::EvexVfnmadd213pdVpdHpdWpd),
                // VFNMADD231PS/PD
                (0xBC, 1, 0) => Some(Opcode::EvexVfnmadd231psVpsHpsWps),
                (0xBC, 1, 1) => Some(Opcode::EvexVfnmadd231pdVpdHpdWpd),
                // VFNMSUB132PS/PD
                (0x9E, 1, 0) => Some(Opcode::EvexVfnmsub132psVpsHpsWps),
                (0x9E, 1, 1) => Some(Opcode::EvexVfnmsub132pdVpdHpdWpd),
                // VFNMSUB213PS/PD
                (0xAE, 1, 0) => Some(Opcode::EvexVfnmsub213psVpsHpsWps),
                (0xAE, 1, 1) => Some(Opcode::EvexVfnmsub213pdVpdHpdWpd),
                // VFNMSUB231PS/PD
                (0xBE, 1, 0) => Some(Opcode::EvexVfnmsub231psVpsHpsWps),
                (0xBE, 1, 1) => Some(Opcode::EvexVfnmsub231pdVpdHpdWpd),

                // --- Compare (avx512_cmp.rs) ---
                // VPTESTMD — EVEX.66.0F38.W0 27
                (0x27, 1, 0) => Some(Opcode::EvexVptestmdKgwHdqWdq),
                // VPTESTMQ — EVEX.66.0F38.W1 27
                (0x27, 1, 1) => Some(Opcode::EvexVptestmqKgbHdqWdq),
                // VPTESTNMD — EVEX.F3.0F38.W0 27
                (0x27, 2, 0) => Some(Opcode::EvexVptestnmdKgwHdqWdq),
                // VPTESTNMQ — EVEX.F3.0F38.W1 27
                (0x27, 2, 1) => Some(Opcode::EvexVptestnmqKgbHdqWdq),
                // VPMOVM2D — EVEX.F3.0F38.W0 38
                (0x38, 2, 0) => Some(Opcode::EvexVpmovm2dVdqKew),
                // VPMOVM2Q — EVEX.F3.0F38.W1 38
                (0x38, 2, 1) => Some(Opcode::EvexVpmovm2qVdqKeb),
                // VPMOVD2M — EVEX.F3.0F38.W0 39
                (0x39, 2, 0) => Some(Opcode::EvexVpmovd2mKgwWdq),
                // VPMOVQ2M — EVEX.F3.0F38.W1 39
                (0x39, 2, 1) => Some(Opcode::EvexVpmovq2mKgbWdq),

                // --- Broadcast (avx512_bcast.rs) ---
                // VBROADCASTSS — EVEX.66.0F38.W0 18
                (0x18, 1, 0) => Some(Opcode::EvexVbroadcastssVpsWss),
                // VBROADCASTSD — EVEX.66.0F38.W1 19
                (0x19, 1, 1) => Some(Opcode::EvexVbroadcastsdVpdWsd),
                // VBROADCASTI32x4 — EVEX.66.0F38.W0 5A
                (0x5A, 1, 0) => Some(Opcode::EvexVbroadcasti32x4VdqWdq),
                // VBROADCASTF32x4 — EVEX.66.0F38.W0 1A
                (0x1A, 1, 0) => Some(Opcode::EvexVbroadcastf32x4VpsWps),
                // VBROADCASTI64x2 — EVEX.66.0F38.W1 5A
                (0x5A, 1, 1) => Some(Opcode::EvexVbroadcasti64x2VdqWdq),
                // VBROADCASTF64x2 — EVEX.66.0F38.W1 1A
                (0x1A, 1, 1) => Some(Opcode::EvexVbroadcastf64x2VpdWpd),
                // VBROADCASTI32x8 — EVEX.66.0F38.W0 5B
                (0x5B, 1, 0) => Some(Opcode::EvexVbroadcasti32x8VdqWdq),
                // VBROADCASTF32x8 — EVEX.66.0F38.W0 1B
                (0x1B, 1, 0) => Some(Opcode::EvexVbroadcastf32x8VpsWps),
                // VBROADCASTI64x4 — EVEX.66.0F38.W1 5B
                (0x5B, 1, 1) => Some(Opcode::EvexVbroadcasti64x4VdqWdq),
                // VBROADCASTF64x4 — EVEX.66.0F38.W1 1B
                (0x1B, 1, 1) => Some(Opcode::EvexVbroadcastf64x4VpdWpd),
                // VPBROADCASTB — EVEX.66.0F38.W0 78
                (0x78, 1, 0) => Some(Opcode::EvexVpbroadcastbVdqWb),
                // VPBROADCASTW — EVEX.66.0F38.W0 79
                (0x79, 1, 0) => Some(Opcode::EvexVpbroadcastwVdqWw),

                // --- Integer (avx512_int.rs) in Map 2 ---
                // VPMULDQ — EVEX.66.0F38.W1 28
                (0x28, 1, 1) => Some(Opcode::EvexVpmuldqVdqHdqWdq),
                // VPMADDUBSW — EVEX.66.0F38.W0 04
                (0x04, 1, 0) => Some(Opcode::EvexVpmaddubswVdqHdqWdq),
                // VPMINUD — EVEX.66.0F38.W0 3B
                (0x3B, 1, 0) => Some(Opcode::EvexVpminudVdqHdqWdq),
                // VPMAXUD — EVEX.66.0F38.W0 3F
                (0x3F, 1, 0) => Some(Opcode::EvexVpmaxudVdqHdqWdq),
                // VPMINUQ — EVEX.66.0F38.W1 3B
                (0x3B, 1, 1) => Some(Opcode::EvexVpminuqVdqHdqWdq),
                // VPMAXUQ — EVEX.66.0F38.W1 3F
                (0x3F, 1, 1) => Some(Opcode::EvexVpmaxuqVdqHdqWdq),
                // VPACKUSDW — EVEX.66.0F38.W0 2B
                (0x2B, 1, 0) => Some(Opcode::EvexVpackusdwVdqHdqWdq),

                // --- Permute (avx512_perm.rs) in Map 2 ---
                // VPERMILPS reg — EVEX.66.0F38.W0 0C
                (0x0C, 1, 0) => Some(Opcode::EvexVpermilpsVpsHpsWps),
                // VPERMPS — EVEX.66.0F38.W0 16
                (0x16, 1, 0) => Some(Opcode::EvexVpermpsVpsHpsWpsKmask),

                // --- Rounding/scale (avx512_round.rs) in Map 2 ---
                // VSCALEFPS — EVEX.66.0F38.W0 2C
                (0x2C, 1, 0) => Some(Opcode::EvexVscalefpsVpsHpsWps),
                // VSCALEFPD — EVEX.66.0F38.W1 2C
                (0x2C, 1, 1) => Some(Opcode::EvexVscalefpdVpdHpdWpd),
                // VGETEXPPS — EVEX.66.0F38.W0 42
                (0x42, 1, 0) => Some(Opcode::EvexVgetexppsVpsWps),
                // VGETEXPPD — EVEX.66.0F38.W1 42
                (0x42, 1, 1) => Some(Opcode::EvexVgetexppdVpdWpd),

                // --- Misc (avx512_misc.rs) in Map 2 ---
                // VPCOMPRESSD — EVEX.66.0F38.W0 8B
                (0x8B, 1, 0) => Some(Opcode::EvexVpcompressdWdqVdq),
                // VPCOMPRESSQ — EVEX.66.0F38.W1 8B
                (0x8B, 1, 1) => Some(Opcode::EvexVpcompressqWdqVdq),
                // VPEXPANDD — EVEX.66.0F38.W0 89
                (0x89, 1, 0) => Some(Opcode::EvexVpexpanddVdqWdq),
                // VPEXPANDQ — EVEX.66.0F38.W1 89
                (0x89, 1, 1) => Some(Opcode::EvexVpexpandqVdqWdq),
                // VPCONFLICTD — EVEX.66.0F38.W0 C4
                (0xC4, 1, 0) => Some(Opcode::EvexVpconflictdVdqWdqKmask),
                // VPLZCNTD — EVEX.66.0F38.W0 44
                (0x44, 1, 0) => Some(Opcode::EvexVplzcntdVdqWdqKmask),
                // VPLZCNTQ — EVEX.66.0F38.W1 44
                (0x44, 1, 1) => Some(Opcode::EvexVplzcntqVdqWdqKmask),
                // VPMOVDB — EVEX.F3.0F38.W0 31
                (0x31, 2, 0) => Some(Opcode::EvexVpmovdbWdqVdq),
                // VPMOVDW — EVEX.F3.0F38.W0 33
                (0x33, 2, 0) => Some(Opcode::EvexVpmovdwWdqVdq),
                // VPMOVQD — EVEX.F3.0F38.W0 35
                (0x35, 2, 0) => Some(Opcode::EvexVpmovqdWdqVdq),

                // --- VPERMI2D — EVEX.66.0F38.W0 76
                (0x76, 1, 0) => Some(Opcode::EvexVpermi2dVdqHdqWdqKmask),

                // --- Gather (avx512_gather.rs) ---
                // VPGATHERDD — EVEX.66.0F38.W0 90
                (0x90, 1, 0) => Some(Opcode::EvexVgatherddVdqVsib),
                // VPGATHERDQ — EVEX.66.0F38.W1 90
                (0x90, 1, 1) => Some(Opcode::EvexVgatherdqVdqVsib),
                // VPGATHERQD — EVEX.66.0F38.W0 91
                (0x91, 1, 0) => Some(Opcode::EvexVgatherqdVdqVsib),
                // VPGATHERQQ — EVEX.66.0F38.W1 91
                (0x91, 1, 1) => Some(Opcode::EvexVgatherqqVdqVsib),

                _ => None,
            }
        }
        3 => {
            // Map 3 (0F 3A xx)
            match (opcode, sse_prefix, w) {
                // VPALIGNR — EVEX.66.0F3A.W0 0F
                (0x0F, 1, 0) => Some(Opcode::EvexVpalignrVdqHdqWdqIb),
                // VPTERNLOGD — EVEX.66.0F3A.W0 25
                (0x25, 1, 0) => Some(Opcode::EvexVpternlogdVdqHdqWdqIb),
                // VPTERNLOGQ — EVEX.66.0F3A.W1 25
                (0x25, 1, 1) => Some(Opcode::EvexVpternlogqVdqHdqWdqIb),
                // VINSERTI32x4 — EVEX.66.0F3A.W0 38
                (0x38, 1, 0) => Some(Opcode::EvexVinserti32x4VdqHdqWdqIb),
                // VINSERTI64x2 — EVEX.66.0F3A.W1 38
                (0x38, 1, 1) => Some(Opcode::EvexVinserti64x2VdqHdqWdqIb),
                // VINSERTF32x4 — EVEX.66.0F3A.W0 18
                (0x18, 1, 0) => Some(Opcode::EvexVinsertf32x4VpsHpsWpsIb),
                // VINSERTF64x2 — EVEX.66.0F3A.W1 18
                (0x18, 1, 1) => Some(Opcode::EvexVinsertf64x2VpdHpdWpdIb),
                // VEXTRACTI32x4 — EVEX.66.0F3A.W0 39
                (0x39, 1, 0) => Some(Opcode::EvexVextracti32x4WdqVdqIb),
                // VEXTRACTI64x2 — EVEX.66.0F3A.W1 39
                (0x39, 1, 1) => Some(Opcode::EvexVextracti64x2WdqVdqIb),
                // VINSERTI32x8 — EVEX.66.0F3A.W0 3A
                (0x3A, 1, 0) => Some(Opcode::EvexVinserti32x8VdqHdqWdqIb),
                // VINSERTI64x4 — EVEX.66.0F3A.W1 3A
                (0x3A, 1, 1) => Some(Opcode::EvexVinserti64x4VdqHdqWdqIb),
                // VINSERTF32x8 — EVEX.66.0F3A.W0 1A
                (0x1A, 1, 0) => Some(Opcode::EvexVinsertf32x8VpsHpsWpsIb),
                // VINSERTF64x4 — EVEX.66.0F3A.W1 1A
                (0x1A, 1, 1) => Some(Opcode::EvexVinsertf64x4VpdHpdWpdIb),
                // VEXTRACTI32x8 — EVEX.66.0F3A.W0 3B
                (0x3B, 1, 0) => Some(Opcode::EvexVextracti32x8WdqVdqIb),
                // VEXTRACTI64x4 — EVEX.66.0F3A.W1 3B
                (0x3B, 1, 1) => Some(Opcode::EvexVextracti64x4WdqVdqIb),
                // VEXTRACTF32x4 — EVEX.66.0F3A.W0 19
                (0x19, 1, 0) => Some(Opcode::EvexVextractf32x4WpsVpsIb),
                // VEXTRACTF64x2 — EVEX.66.0F3A.W1 19
                (0x19, 1, 1) => Some(Opcode::EvexVextractf64x2WpdVpdIb),
                // VEXTRACTF32x8 — EVEX.66.0F3A.W0 1B
                (0x1B, 1, 0) => Some(Opcode::EvexVextractf32x8WpsVpsIb),
                // VEXTRACTF64x4 — EVEX.66.0F3A.W1 1B
                (0x1B, 1, 1) => Some(Opcode::EvexVextractf64x4WpdVpdIb),
                // VPERMQ imm — EVEX.66.0F3A.W1 00
                (0x00, 1, 1) => Some(Opcode::EvexVpermqVdqWdqIbKmask),
                // VPERMPD imm — EVEX.66.0F3A.W1 01
                (0x01, 1, 1) => Some(Opcode::EvexVpermpdVpdWpdIbKmask),
                // VPCMPD — EVEX.66.0F3A.W0 1F
                (0x1F, 1, 0) => Some(Opcode::EvexVpcmpdKgwHdqWdqIb),
                // VPCMPUD — EVEX.66.0F3A.W0 1E
                (0x1E, 1, 0) => Some(Opcode::EvexVpcmpudKgwHdqWdqIb),
                // VCMPPS — EVEX.0F3A C2 already in Map 1 above
                // VRNDSCALEPS — EVEX.66.0F3A.W0 08
                (0x08, 1, 0) => Some(Opcode::EvexVrndscalepsVpsWpsIbKmask),
                // VRNDSCALEPD — EVEX.66.0F3A.W1 09
                (0x09, 1, 1) => Some(Opcode::EvexVrndscalepdVpdWpdIbKmask),
                // VRNDSCALESS — EVEX.66.0F3A.W0 0A
                (0x0A, 1, 0) => Some(Opcode::EvexVrndscalessVssHpsWssIbKmask),
                // VRNDSCALESD — EVEX.66.0F3A.W1 0B
                (0x0B, 1, 1) => Some(Opcode::EvexVrndscalesdVsdHpdWsdIbKmask),
                // VGETMANTPS — EVEX.66.0F3A.W0 26
                (0x26, 1, 0) => Some(Opcode::EvexVgetmantpsVpsWpsIbKmask),
                // VGETMANTPD — EVEX.66.0F3A.W1 26
                (0x26, 1, 1) => Some(Opcode::EvexVgetmantpdVpdWpdIbKmask),
                // VPERMILPS imm — EVEX.66.0F3A.W0 04
                (0x04, 1, 0) => Some(Opcode::EvexVpermilpsVpsWpsIb),
                // VPERMILPD imm — EVEX.66.0F3A.W1 05
                (0x05, 1, 1) => Some(Opcode::EvexVpermilpdVpdWpdIb),
                // VSHUFPS — already in Map 1
                // VSHUFF32x4 — EVEX.66.0F3A.W0 23
                (0x23, 1, 0) => Some(Opcode::EvexVshuff32x4VpsHpsWpsIbKmask),
                // VSHUFF64x2 — EVEX.66.0F3A.W1 23
                (0x23, 1, 1) => Some(Opcode::EvexVshuff64x2VpdHpdWpdIbKmask),
                // VSHUFI32x4 — EVEX.66.0F3A.W0 43
                (0x43, 1, 0) => Some(Opcode::EvexVshufi32x4VdqHdqWdqIbKmask),
                // VSHUFI64x2 — EVEX.66.0F3A.W1 43
                (0x43, 1, 1) => Some(Opcode::EvexVshufi64x2VdqHdqWdqIbKmask),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Remap SSE opcodes to VEX opcodes when VEX prefix is active.
///
/// The opcode tables are shared between SSE and VEX instructions. When a VEX
/// prefix is present, the table lookup may return an SSE opcode (2-operand form
/// that ignores VEX.vvvv). This function remaps to the proper VEX opcode so the
/// 3-operand VEX handler is dispatched.
///
/// `vl`: VEX.L field — 0 = 128-bit (XMM), 1 = 256-bit (YMM)
const fn remap_sse_to_vex(op: Opcode, vl: u8) -> Opcode {
    use Opcode::*;
    match op {
        // ===== Integer arithmetic =====
        PadddVdqWdq   => if vl == 0 { V128VpadddVdqHdqWdq }   else { V256VpadddVdqHdqWdq },
        PaddqVdqWdq   => if vl == 0 { V128VpaddqVdqHdqWdq }   else { V256VpaddqVdqHdqWdq },
        PaddwVdqWdq   => if vl == 0 { V128VpaddwVdqHdqWdq }   else { V256VpaddwVdqHdqWdq },
        PaddbVdqWdq   => if vl == 0 { V128VpaddbVdqHdqWdq }   else { V256VpaddbVdqHdqWdq },
        PsubdVdqWdq   => if vl == 0 { V128VpsubdVdqHdqWdq }   else { V256VpsubdVdqHdqWdq },
        PsubqVdqWdq   => if vl == 0 { V128VpsubqVdqHdqWdq }   else { V256VpsubqVdqHdqWdq },
        PsubwVdqWdq   => if vl == 0 { V128VpsubwVdqHdqWdq }   else { V256VpsubwVdqHdqWdq },
        PsubbVdqWdq   => if vl == 0 { V128VpsubbVdqHdqWdq }   else { V256VpsubbVdqHdqWdq },
        // Saturating
        PaddsbVdqWdq  => if vl == 0 { V128VpaddsbVdqHdqWdq }  else { V256VpaddsbVdqHdqWdq },
        PaddswVdqWdq  => if vl == 0 { V128VpaddswVdqHdqWdq }  else { V256VpaddswVdqHdqWdq },
        PsubsbVdqWdq  => if vl == 0 { V128VpsubsbVdqHdqWdq }  else { V256VpsubsbVdqHdqWdq },
        PsubswVdqWdq  => if vl == 0 { V128VpsubswVdqHdqWdq }  else { V256VpsubswVdqHdqWdq },
        PsubusbVdqWdq => if vl == 0 { V128VpsubusbVdqHdqWdq } else { V256VpsubusbVdqHdqWdq },
        PsubuswVdqWdq => if vl == 0 { V128VpsubuswVdqHdqWdq } else { V256VpsubuswVdqHdqWdq },
        PaddusbVdqWdq => if vl == 0 { V128VpaddusbVdqHdqWdq } else { V256VpaddusbVdqHdqWdq },
        PadduswVdqWdq => if vl == 0 { V128VpadduswVdqHdqWdq } else { V256VpadduswVdqHdqWdq },

        // ===== Logical =====
        PxorVdqWdq  => if vl == 0 { V128VpxorVdqHdqWdq }  else { V256VpxorVdqHdqWdq },
        PandVdqWdq  => if vl == 0 { V128VpandVdqHdqWdq }  else { V256VpandVdqHdqWdq },
        PorVdqWdq   => if vl == 0 { V128VporVdqHdqWdq }   else { V256VporVdqHdqWdq },
        PandnVdqWdq => if vl == 0 { V128VpandnVdqHdqWdq } else { V256VpandnVdqHdqWdq },

        // ===== Multiply =====
        PmuludqVdqWdq => if vl == 0 { V128VpmuludqVdqHdqWdq } else { V256VpmuludqVdqHdqWdq },
        PmuldqVdqWdq  => if vl == 0 { V128VpmuldqVdqHdqWdq }  else { V256VpmuldqVdqHdqWdq },
        PmulldVdqWdq  => if vl == 0 { V128VpmulldVdqHdqWdq }  else { V256VpmulldVdqHdqWdq },
        PmullwVdqWdq  => if vl == 0 { V128VpmullwVdqHdqWdq }  else { V256VpmullwVdqHdqWdq },
        PmulhwVdqWdq  => if vl == 0 { V128VpmulhwVdqHdqWdq }  else { V256VpmulhwVdqHdqWdq },
        PmulhuwVdqWdq => if vl == 0 { V128VpmulhuwVdqHdqWdq } else { V256VpmulhuwVdqHdqWdq },
        PmulhrswVdqWdq=> if vl == 0 { V128VpmulhrswVdqHdqWdq }else { V256VpmulhrswVdqHdqWdq },

        // ===== Compare =====
        PcmpeqbVdqWdq => if vl == 0 { V128VpcmpeqbVdqHdqWdq } else { V256VpcmpeqbVdqHdqWdq },
        PcmpeqwVdqWdq => if vl == 0 { V128VpcmpeqwVdqHdqWdq } else { V256VpcmpeqwVdqHdqWdq },
        PcmpeqdVdqWdq => if vl == 0 { V128VpcmpeqdVdqHdqWdq } else { V256VpcmpeqdVdqHdqWdq },
        PcmpeqqVdqWdq => if vl == 0 { V128VpcmpeqqVdqHdqWdq } else { V256VpcmpeqqVdqHdqWdq },
        PcmpgtbVdqWdq => if vl == 0 { V128VpcmpgtbVdqHdqWdq } else { V256VpcmpgtbVdqHdqWdq },
        PcmpgtwVdqWdq => if vl == 0 { V128VpcmpgtwVdqHdqWdq } else { V256VpcmpgtwVdqHdqWdq },
        PcmpgtdVdqWdq => if vl == 0 { V128VpcmpgtdVdqHdqWdq } else { V256VpcmpgtdVdqHdqWdq },
        PcmpgtqVdqWdq => if vl == 0 { V128VpcmpgtqVdqHdqWdq } else { V256VpcmpgtqVdqHdqWdq },

        // ===== Shift by register =====
        PsrlwVdqWdq => if vl == 0 { V128VpsrlwVdqHdqWdq } else { V256VpsrlwVdqHdqWdq },
        PsrldVdqWdq => if vl == 0 { V128VpsrldVdqHdqWdq } else { V256VpsrldVdqHdqWdq },
        PsrlqVdqWdq => if vl == 0 { V128VpsrlqVdqHdqWdq } else { V256VpsrlqVdqHdqWdq },
        PsrawVdqWdq => if vl == 0 { V128VpsrawVdqHdqWdq } else { V256VpsrawVdqHdqWdq },
        PsradVdqWdq => if vl == 0 { V128VpsradVdqHdqWdq } else { V256VpsradVdqHdqWdq },
        PsllwVdqWdq => if vl == 0 { V128VpsllwVdqHdqWdq } else { V256VpsllwVdqHdqWdq },
        PslldVdqWdq => if vl == 0 { V128VpslldVdqHdqWdq } else { V256VpslldVdqHdqWdq },
        PsllqVdqWdq => if vl == 0 { V128VpsllqVdqHdqWdq } else { V256VpsllqVdqHdqWdq },

        // ===== Shift by immediate (Group 12/13/14) =====
        PsrlwUdqIb  => if vl == 0 { V128VpsrlwUdqIb }  else { V256VpsrlwUdqIb },
        PsrldUdqIb  => if vl == 0 { V128VpsrldUdqIb }  else { V256VpsrldUdqIb },
        PsrlqUdqIb  => if vl == 0 { V128VpsrlqUdqIb }  else { V256VpsrlqUdqIb },
        PsrawUdqIb  => if vl == 0 { V128VpsrawUdqIb }  else { V256VpsrawUdqIb },
        PsradUdqIb  => if vl == 0 { V128VpsradUdqIb }  else { V256VpsradUdqIb },
        PsllwUdqIb  => if vl == 0 { V128VpsllwUdqIb }  else { V256VpsllwUdqIb },
        PslldUdqIb  => if vl == 0 { V128VpslldUdqIb }  else { V256VpslldUdqIb },
        PsllqUdqIb  => if vl == 0 { V128VpsllqUdqIb }  else { V256VpsllqUdqIb },
        PsrldqUdqIb => if vl == 0 { V128VpsrldqUdqIb } else { V256VpsrldqUdqIb },
        PslldqUdqIb => if vl == 0 { V128VpslldqUdqIb } else { V256VpslldqUdqIb },

        // ===== Shuffle / Unpack =====
        PshufbVdqWdq     => if vl == 0 { V128VpshufbVdqHdqWdq }     else { V256VpshufbVdqHdqWdq },
        PshufdVdqWdqIb   => if vl == 0 { V128VpshufdVdqWdqIb }      else { V256VpshufdVdqWdqIb },
        PshufhwVdqWdqIb  => if vl == 0 { V128VpshufhwVdqWdqIb }     else { V256VpshufhwVdqWdqIb },
        PshuflwVdqWdqIb  => if vl == 0 { V128VpshuflwVdqWdqIb }     else { V256VpshuflwVdqWdqIb },
        PunpckldqVdqWdq  => if vl == 0 { V128VpunpckldqVdqHdqWdq }  else { V256VpunpckldqVdqHdqWdq },
        PunpckhdqVdqWdq  => if vl == 0 { V128VpunpckhdqVdqHdqWdq }  else { V256VpunpckhdqVdqHdqWdq },
        PunpcklbwVdqWdq  => if vl == 0 { V128VpunpcklbwVdqHdqWdq }  else { V256VpunpcklbwVdqHdqWdq },
        PunpckhbwVdqWdq  => if vl == 0 { V128VpunpckhbwVdqHdqWdq }  else { V256VpunpckhbwVdqHdqWdq },
        PunpcklwdVdqWdq  => if vl == 0 { V128VpunpcklwdVdqHdqWdq }  else { V256VpunpcklwdVdqHdqWdq },
        PunpckhwdVdqWdq  => if vl == 0 { V128VpunpckhwdVdqHdqWdq }  else { V256VpunpckhwdVdqHdqWdq },
        PunpcklqdqVdqWdq => if vl == 0 { V128VpunpcklqdqVdqHdqWdq } else { V256VpunpcklqdqVdqHdqWdq },
        PunpckhqdqVdqWdq => if vl == 0 { V128VpunpckhqdqVdqHdqWdq } else { V256VpunpckhqdqVdqHdqWdq },

        // ===== PALIGNR =====
        PalignrVdqWdqIb => if vl == 0 { V128VpalignrVdqHdqWdqIb } else { V256VpalignrVdqHdqWdqIb },

        // ===== Pack =====
        PacksswbVdqWdq  => if vl == 0 { V128VpacksswbVdqHdqWdq }  else { V256VpacksswbVdqHdqWdq },
        PackuswbVdqWdq  => if vl == 0 { V128VpackuswbVdqHdqWdq }  else { V256VpackuswbVdqHdqWdq },
        PackssdwVdqWdq  => if vl == 0 { V128VpackssdwVdqHdqWdq }  else { V256VpackssdwVdqHdqWdq },
        PackusdwVdqWdq  => if vl == 0 { V128VpackusdwVdqHdqWdq }  else { V256VpackusdwVdqHdqWdq },

        // ===== Min/Max (SSE2 + SSE4.1) =====
        PminubVdqWdq  => if vl == 0 { V128VpminubVdqHdqWdq }  else { V256VpminubVdqHdqWdq },
        PminswVdqWdq  => if vl == 0 { V128VpminswVdqHdqWdq }  else { V256VpminswVdqHdqWdq },
        PmaxubVdqWdq  => if vl == 0 { V128VpmaxubVdqHdqWdq }  else { V256VpmaxubVdqHdqWdq },
        PmaxswVdqWdq  => if vl == 0 { V128VpmaxswVdqHdqWdq }  else { V256VpmaxswVdqHdqWdq },
        PminsbVdqWdq  => if vl == 0 { V128VpminsbVdqHdqWdq }  else { V256VpminsbVdqHdqWdq },
        PminsdVdqWdq  => if vl == 0 { V128VpminsdVdqHdqWdq }  else { V256VpminsdVdqHdqWdq },
        PminuwVdqWdq  => if vl == 0 { V128VpminuwVdqHdqWdq }  else { V256VpminuwVdqHdqWdq },
        PminudVdqWdq  => if vl == 0 { V128VpminudVdqHdqWdq }  else { V256VpminudVdqHdqWdq },
        PmaxsbVdqWdq  => if vl == 0 { V128VpmaxsbVdqHdqWdq }  else { V256VpmaxsbVdqHdqWdq },
        PmaxsdVdqWdq  => if vl == 0 { V128VpmaxsdVdqHdqWdq }  else { V256VpmaxsdVdqHdqWdq },
        PmaxuwVdqWdq  => if vl == 0 { V128VpmaxuwVdqHdqWdq }  else { V256VpmaxuwVdqHdqWdq },
        PmaxudVdqWdq  => if vl == 0 { V128VpmaxudVdqHdqWdq }  else { V256VpmaxudVdqHdqWdq },

        // ===== Average / SAD =====
        PavgbVdqWdq   => if vl == 0 { V128VpavgbVdqWdq }      else { V256VpavgbVdqWdq },
        PavgwVdqWdq   => if vl == 0 { V128VpavgwVdqWdq }      else { V256VpavgwVdqWdq },
        PsadbwVdqWdq  => if vl == 0 { V128VpsadbwVdqHdqWdq }  else { V256VpsadbwVdqHdqWdq },

        // ===== PMADDWD / PMADDUBSW =====
        PmaddwdVdqWdq   => if vl == 0 { V128VpmaddwdVdqHdqWdq }   else { V256VpmaddwdVdqHdqWdq },
        PmaddubswVdqWdq => if vl == 0 { V128VpmaddubswVdqHdqWdq } else { V256VpmaddubswVdqHdqWdq },

        // ===== SSSE3: PHADD/PHSUB/PSIGN =====
        PhaddwVdqWdq   => if vl == 0 { V128VphaddwVdqHdqWdq }   else { V256VphaddwVdqHdqWdq },
        PhadddVdqWdq   => if vl == 0 { V128VphadddVdqHdqWdq }   else { V256VphadddVdqHdqWdq },
        PhaddswVdqWdq  => if vl == 0 { V128VphaddswVdqHdqWdq }  else { V256VphaddswVdqHdqWdq },
        PhsubwVdqWdq   => if vl == 0 { V128VphsubwVdqHdqWdq }   else { V256VphsubwVdqHdqWdq },
        PhsubdVdqWdq   => if vl == 0 { V128VphsubdVdqHdqWdq }   else { V256VphsubdVdqHdqWdq },
        PhsubswVdqWdq  => if vl == 0 { V128VphsubswVdqHdqWdq }  else { V256VphsubswVdqHdqWdq },
        PsignbVdqWdq   => if vl == 0 { V128VpsignbVdqHdqWdq }   else { V256VpsignbVdqHdqWdq },
        PsignwVdqWdq   => if vl == 0 { V128VpsignwVdqHdqWdq }   else { V256VpsignwVdqHdqWdq },
        PsigndVdqWdq   => if vl == 0 { V128VpsigndVdqHdqWdq }   else { V256VpsigndVdqHdqWdq },

        // ===== Floating-point bitwise (VEX handler checks get_vl()) =====
        AndpsVpsWps   => VandpsVpsHpsWps,
        AndnpsVpsWps  => VandnpsVpsHpsWps,
        OrpsVpsWps    => VorpsVpsHpsWps,
        XorpsVpsWps   => VxorpsVpsHpsWps,
        AddpsVpsWps   => VaddpsVpsHpsWps,
        MulpsVpsWps   => VmulpsVpsHpsWps,
        SubpsVpsWps   => VsubpsVpsHpsWps,
        DivpsVpsWps   => VdivpsVpsHpsWps,
        AndpdVpdWpd   => VandpdVpdHpdWpd,
        AndnpdVpdWpd  => VandnpdVpdHpdWpd,
        OrpdVpdWpd    => VorpdVpdHpdWpd,
        XorpdVpdWpd   => VxorpdVpdHpdWpd,
        AddpdVpdWpd   => VaddpdVpdHpdWpd,
        MulpdVpdWpd   => VmulpdVpdHpdWpd,
        SubpdVpdWpd   => VsubpdVpdHpdWpd,
        DivpdVpdWpd   => VdivpdVpdHpdWpd,

        // ===== Store-form moves (VEX handler does VL-aware stores + register form) =====
        MovdquWdqVdq  => if vl == 0 { V128VmovdquWdqVdq }  else { V256VmovdquWdqVdq },
        MovdqaWdqVdq  => if vl == 0 { V128VmovdqaWdqVdq }  else { V256VmovdqaWdqVdq },
        MovupsWpsVps  => if vl == 0 { V128VmovupsWpsVps }   else { V256VmovupsWpsVps },
        MovapsWpsVps  => if vl == 0 { V128VmovapsWpsVps }   else { V256VmovapsWpsVps },
        MovupdWpdVpd  => if vl == 0 { V128VmovupdWpdVpd }   else { V256VmovupdWpdVpd },
        MovapdWpdVpd  => if vl == 0 { V128VmovapdWpdVpd }   else { V256VmovapdWpdVpd },
        MovntdqMdqVdq => if vl == 0 { V128VmovntdqMdqVdq }  else { V256VmovntdqMdqVdq },
        MovntpsMpsVps => if vl == 0 { V128VmovntpsMpsVps }   else { V256VmovntpsMpsVps },
        MovntpdMpdVpd => if vl == 0 { V128VmovntpdMpdVpd }   else { V256VmovntpdMpdVpd },

        // ===== Load-form moves (SSE handler only reads 128-bit; VEX handler is VL-aware) =====
        // These use a single VEX opcode (no V128/V256 prefix) — handler checks get_vl()
        MovdquVdqWdq  => VmovdquVdqWdq,
        MovdqaVdqWdq  => VmovdqaVdqWdq,
        MovupsVpsWps  => VmovupsVpsWps,
        MovapsVpsWps  => VmovapsVpsWps,
        MovupdVpdWpd  => VmovupdVpdWpd,
        MovapdVpdWpd  => VmovapdVpdWpd,

        // ===== Misc =====
        PmovmskbGdUdq => if vl == 0 { V128VpmovmskbGdUdq } else { V256VpmovmskbGdUdq },

        // ===== EMMS → VZEROUPPER/VZEROALL (VEX.0F 77) =====
        Emms => if vl == 0 { Vzeroupper } else { Vzeroall },

        // No remap — instruction either has no VEX form, is already VEX, or
        // works correctly as-is (e.g. 2-operand loads where VEX.vvvv must be 1111b)
        _ => op,
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
        // Multi-byte NOPs (0F 19-1F) — Bochs uses BxOpcodeTableMultiByteNOP
        0x19 => &BxOpcodeTableMultiByteNOP,
        0x1A => &BxOpcodeTableMultiByteNOP,
        0x1B => &BxOpcodeTableMultiByteNOP,
        0x1C => &BxOpcodeTableMultiByteNOP,
        0x1D => &BxOpcodeTableMultiByteNOP,
        0x1E => &BxOpcodeTable0F1E,
        0x1F => &BxOpcodeTableMultiByteNOP,
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
        // SSE data movement, arithmetic, comparison, shuffle (0F 50-7F)
        0x50 => &BxOpcodeTable0F50,
        0x51 => &BxOpcodeTable0F51,
        0x52 => &BxOpcodeTable0F52,
        0x53 => &BxOpcodeTable0F53,
        0x54 => &BxOpcodeTable0F54,
        0x55 => &BxOpcodeTable0F55,
        0x56 => &BxOpcodeTable0F56,
        0x57 => &BxOpcodeTable0F57,
        0x58 => &BxOpcodeTable0F58,
        0x59 => &BxOpcodeTable0F59,
        0x5A => &BxOpcodeTable0F5A,
        0x5B => &BxOpcodeTable0F5B,
        0x5C => &BxOpcodeTable0F5C,
        0x5D => &BxOpcodeTable0F5D,
        0x5E => &BxOpcodeTable0F5E,
        0x5F => &BxOpcodeTable0F5F,
        0x60 => &BxOpcodeTable0F60,
        0x61 => &BxOpcodeTable0F61,
        0x62 => &BxOpcodeTable0F62,
        0x63 => &BxOpcodeTable0F63,
        0x64 => &BxOpcodeTable0F64,
        0x65 => &BxOpcodeTable0F65,
        0x66 => &BxOpcodeTable0F66,
        0x67 => &BxOpcodeTable0F67,
        0x68 => &BxOpcodeTable0F68,
        0x69 => &BxOpcodeTable0F69,
        0x6A => &BxOpcodeTable0F6A,
        0x6B => &BxOpcodeTable0F6B,
        0x6C => &BxOpcodeTable0F6C,
        0x6D => &BxOpcodeTable0F6D,
        0x6E => &BxOpcodeTable0F6E,
        0x6F => &BxOpcodeTable0F6F,
        0x70 => &BxOpcodeTable0F70,
        0x71 => &BxOpcodeTable0F71,
        0x72 => &BxOpcodeTable0F72,
        0x73 => &BxOpcodeTable0F73,
        0x74 => &BxOpcodeTable0F74,
        0x75 => &BxOpcodeTable0F75,
        0x76 => &BxOpcodeTable0F76,
        0x77 => &BxOpcodeTable0F77,
        0x78 => &BxOpcodeTable0F78,
        0x79 => &BxOpcodeTable0F79,
        // 0x7A, 0x7B are UD in Bochs
        0x7C => &BxOpcodeTable0F7C,
        0x7D => &BxOpcodeTable0F7D,
        0x7E => &BxOpcodeTable0F7E,
        0x7F => &BxOpcodeTable0F7F,
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
        // SSE/MMX data operations (0F D0-FE)
        0xD0 => &BxOpcodeTable0FD0,
        0xD1 => &BxOpcodeTable0FD1,
        0xD2 => &BxOpcodeTable0FD2,
        0xD3 => &BxOpcodeTable0FD3,
        0xD4 => &BxOpcodeTable0FD4,
        0xD5 => &BxOpcodeTable0FD5,
        0xD6 => &BxOpcodeTable0FD6,
        0xD7 => &BxOpcodeTable0FD7,
        0xD8 => &BxOpcodeTable0FD8,
        0xD9 => &BxOpcodeTable0FD9,
        0xDA => &BxOpcodeTable0FDA,
        0xDB => &BxOpcodeTable0FDB,
        0xDC => &BxOpcodeTable0FDC,
        0xDD => &BxOpcodeTable0FDD,
        0xDE => &BxOpcodeTable0FDE,
        0xDF => &BxOpcodeTable0FDF,
        0xE0 => &BxOpcodeTable0FE0,
        0xE1 => &BxOpcodeTable0FE1,
        0xE2 => &BxOpcodeTable0FE2,
        0xE3 => &BxOpcodeTable0FE3,
        0xE4 => &BxOpcodeTable0FE4,
        0xE5 => &BxOpcodeTable0FE5,
        0xE6 => &BxOpcodeTable0FE6,
        0xE7 => &BxOpcodeTable0FE7,
        0xE8 => &BxOpcodeTable0FE8,
        0xE9 => &BxOpcodeTable0FE9,
        0xEA => &BxOpcodeTable0FEA,
        0xEB => &BxOpcodeTable0FEB,
        0xEC => &BxOpcodeTable0FEC,
        0xED => &BxOpcodeTable0FED,
        0xEE => &BxOpcodeTable0FEE,
        0xEF => &BxOpcodeTable0FEF,
        0xF0 => &BxOpcodeTable0FF0,
        0xF1 => &BxOpcodeTable0FF1,
        0xF2 => &BxOpcodeTable0FF2,
        0xF3 => &BxOpcodeTable0FF3,
        0xF4 => &BxOpcodeTable0FF4,
        0xF5 => &BxOpcodeTable0FF5,
        0xF6 => &BxOpcodeTable0FF6,
        0xF7 => &BxOpcodeTable0FF7,
        0xF8 => &BxOpcodeTable0FF8,
        0xF9 => &BxOpcodeTable0FF9,
        0xFA => &BxOpcodeTable0FFA,
        0xFB => &BxOpcodeTable0FFB,
        0xFC => &BxOpcodeTable0FFC,
        0xFD => &BxOpcodeTable0FFD,
        0xFE => &BxOpcodeTable0FFE,
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

            // CALL/JMP rel32 — always 4-byte displacement in 64-bit mode
            // (0x66 prefix is ignored for near branches per Intel SDM)
            0xE8 | 0xE9 => 4,

            // Iv/Id (operand-size dependent)
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D | 0x68 | 0x69 | 0xA9
            | 0x81 | 0xC7 => {
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
        // 0x90 is NOP (Bochs decoder64_nop returns NOP for bare 0x90)
        let i = fetch_decode64(&[0x90]).unwrap();
        assert_eq!(i.ilen(), 1);
        assert_eq!(i.get_ia_opcode(), Opcode::Nop);
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
        assert_eq!(i.displacement, 0x10);
    }

    #[test]
    fn test_disp32() {
        let i = fetch_decode64(&[0x8B, 0x83, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.displacement, 0x12345678);
    }

    #[test]
    fn test_imm8() {
        let i = fetch_decode64(&[0x6A, 0x42]).unwrap(); // PUSH 0x42
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.immediate, 0x42);
    }

    #[test]
    fn test_imm32() {
        init_tracing();
        let i = fetch_decode64(&[0x68, 0x78, 0x56, 0x34, 0x12]).unwrap(); // PUSH 0x12345678
        tracing::info!("{i:#x?}");
        let i2 = fetch_decode64(&[0x68, 0x78, 0x56, 0x34, 0x12]); // PUSH 0x12345678
        tracing::info!("{i2:#x?}");
        assert_eq!(i.ilen(), 5);
        assert_eq!(i.immediate, 0x12345678);
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
        assert_eq!(i.immediate, 0x12345678);
    }

    #[test]
    fn test_lock() {
        let i = fetch_decode64(&[0xF0, 0x87, 0x03]).unwrap(); // LOCK XCHG
        assert_eq!(i.ilen(), 3);
        // Check raw bits - expect bit 6 set for LOCK
        let bits = i.flags.bits();
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

        for (_, instruction) in &instructions {
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
    ) -> Vec<(u64, Instruction)> {
        let mut offset = 0;
        let mut current_address = runtime_address;
        let mut instructions = Vec::new();

        while offset < data.len() {
            let remaining = &data[offset..];

            let decoded = match fetch_decode64(remaining) {
                Ok(instr) => instr,
                Err(_) => break,
            };

            let length = decoded.length as usize;

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
            assert_eq!(instr.operands.src1, seg); // Source segment register

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
            assert_eq!(instr.operands.dst, seg); // Destination segment register
        }
    }

    /// Test x87 FPU escape opcodes (D8-DF) decode correctly in 64-bit mode
    #[test]
    fn test_fpu_escape_64() {
        // D8 C0 = FADD ST(0), ST(0) — register form (mod=11, modrm & 0x3F = 0x00, index = 0+8 = 8)
        let i = fetch_decode64(&[0xD8, 0xC0]).unwrap();
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.get_ia_opcode(), Opcode::FaddSt0Stj);

        // D9 E8 = FLD1 — register form (mod=11, modrm & 0x3F = 0x28, index = 0x28+8 = 48)
        let i = fetch_decode64(&[0xD9, 0xE8]).unwrap();
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.get_ia_opcode(), Opcode::FLD1);

        // DD 05 00 00 00 00 = FLD QWORD [RIP+0] — memory form (mod=00, nnn=0)
        let i = fetch_decode64(&[0xDD, 0x05, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.get_ia_opcode(), Opcode::FldDoubleReal);

        // DE C1 = FADDP ST(1), ST(0) — register form
        let i = fetch_decode64(&[0xDE, 0xC1]).unwrap();
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.get_ia_opcode(), Opcode::FaddpStiSt0);

        // Verify foo field: (modrm | (escape_byte << 8)) & 0x7FF
        let i = fetch_decode64(&[0xD8, 0xC0]).unwrap();
        let expected_foo = ((0xC0u16) | (0xD8u16 << 8)) & 0x7FF;
        assert_eq!(i.id() as u16, expected_foo);
    }

    /// Test MOV CR/DR forces ModC0 in 64-bit mode
    #[test]
    fn test_mov_cr_force_modc0() {
        // 0F 20 C0 = MOV RAX, CR0 (mod=11, nnn=0, rm=0)
        let i = fetch_decode64(&[0x0F, 0x20, 0xC0]).unwrap();
        assert_eq!(i.ilen(), 3);
        assert!(i.mod_c0());

        // 0F 20 00 = MOV RAX, CR0 with mod=00 (should still be treated as register)
        let i = fetch_decode64(&[0x0F, 0x20, 0x00]).unwrap();
        assert!(i.mod_c0(), "MOV CR should force ModC0 even with mod=00");
    }

    /// Test UD64 opcodes are rejected in 64-bit mode
    #[test]
    fn test_ud64_opcodes() {
        // PUSH ES (0x06) — invalid in 64-bit mode
        assert!(fetch_decode64(&[0x06]).is_err());
        // POP ES (0x07)
        assert!(fetch_decode64(&[0x07]).is_err());
        // PUSH CS (0x0E)
        assert!(fetch_decode64(&[0x0E]).is_err());
        // DAA (0x27)
        assert!(fetch_decode64(&[0x27]).is_err());
        // DAS (0x2F)
        assert!(fetch_decode64(&[0x2F]).is_err());
        // AAA (0x37)
        assert!(fetch_decode64(&[0x37]).is_err());
        // AAS (0x3F)
        assert!(fetch_decode64(&[0x3F]).is_err());
        // PUSHA (0x60)
        assert!(fetch_decode64(&[0x60]).is_err());
        // POPA (0x61)
        assert!(fetch_decode64(&[0x61]).is_err());
        // INTO (0xCE)
        assert!(fetch_decode64(&[0xCE]).is_err());
        // AAM (0xD4)
        assert!(fetch_decode64(&[0xD4, 0x0A]).is_err());
        // SALC (0xD6)
        assert!(fetch_decode64(&[0xD6]).is_err());
        // Two-byte: 0F 24 (invalid in 64-bit)
        assert!(fetch_decode64(&[0x0F, 0x24, 0xC0]).is_err());
    }

    /// Test LOCK prefix validation
    #[test]
    fn test_lock_register_rejected() {
        // LOCK ADD EAX, EBX (F0 01 D8) — LOCK on register form → #UD
        assert!(fetch_decode64(&[0xF0, 0x01, 0xD8]).is_err());
        // LOCK ADD [RAX], EBX (F0 01 18) — LOCK on memory form → OK
        assert!(fetch_decode64(&[0xF0, 0x01, 0x18]).is_ok());
    }

    /// Test segment defaults for RSP/RBP base in 64-bit mode
    #[test]
    fn test_segment_defaults_64() {
        // MOV EAX, [RSP] (8B 04 24) — SIB with base=RSP → SS
        let i = fetch_decode64(&[0x8B, 0x04, 0x24]).unwrap();
        assert_eq!(i.operands.segment, 2); // SEG = SS

        // MOV EAX, [RBP+0] (8B 45 00) — mod=1, base=RBP → SS
        let i = fetch_decode64(&[0x8B, 0x45, 0x00]).unwrap();
        assert_eq!(i.operands.segment, 2); // SEG = SS

        // MOV EAX, [RAX] (8B 00) — base=RAX → DS
        let i = fetch_decode64(&[0x8B, 0x00]).unwrap();
        assert_eq!(i.operands.segment, 3); // SEG = DS
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
