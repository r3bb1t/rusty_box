//! 32-bit / 16-bit instruction decoder (matching Bochs `fetchdecode32.cc`).
//!
//! Provides `fetch_decode32` and `fetch_decode32_inplace` — both are `const fn`
//! decoders for x86 protected/real mode that produce an [`Instruction`].

use crate::instruction::{Instruction, InstructionFlags};
use crate::error::{DecodeError, DecodeResult};
use crate::opcode::Opcode;
use crate::BxSegregs;

use super::tables::{BxDecodeError, SsePrefix};

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

// Decoding mask bit offsets
use super::tables::{
    AS32_OFFSET, LOCK_PREFIX_OFFSET, MODC0_OFFSET, NNN_OFFSET, OS32_OFFSET, RRR_OFFSET,
    SRC_EQ_DST_OFFSET, SSE_PREFIX_OFFSET,
};

/// Search opcode table for matching opcode
const fn find_opcode_in_table(table: &[u64], decmask: u32) -> Opcode {
    let mut i = 0;
    while i < table.len() {
        let entry = table[i];
        let ignmsk = (entry & 0xFFFFFF) as u32;
        let opmsk = (entry >> 24) as u32;

        if (opmsk & ignmsk) == (decmask & ignmsk) {
            let opcode_raw = ((entry >> 48) & 0x7FFF) as u16;
            return Opcode::from_u16_const(opcode_raw);
        }

        if (entry as i64) < 0 {
            break;
        }

        i += 1;
    }
    Opcode::IaError
}

// Backward-compatible aliases for MetaInfoFlags
use InstructionFlags as MetaInfoFlags;

// 16-bit register indices — matching Bochs BX_16BIT_REG_* constants
const BX_16BIT_REG_BX: u8 = 3;
const BX_16BIT_REG_BP: u8 = 5;
const BX_16BIT_REG_SI: u8 = 6;
const BX_16BIT_REG_DI: u8 = 7;
const BX_NIL_REGISTER: u8 = 19;
const BX_NO_INDEX: u8 = 4; // ESP index = no index in SIB

// 16-bit addressing mode base registers — Bochs fetchdecode32.cc Resolve16BaseReg[]
const RESOLVE16_BASE_REG: [u8; 8] = [
    BX_16BIT_REG_BX,
    BX_16BIT_REG_BX,
    BX_16BIT_REG_BP,
    BX_16BIT_REG_BP,
    BX_16BIT_REG_SI,
    BX_16BIT_REG_DI,
    BX_16BIT_REG_BP,
    BX_16BIT_REG_BX,
];

// 16-bit addressing mode index registers — Bochs fetchdecode32.cc Resolve16IndexReg[]
const RESOLVE16_INDEX_REG: [u8; 8] = [
    BX_16BIT_REG_SI,
    BX_16BIT_REG_DI,
    BX_16BIT_REG_SI,
    BX_16BIT_REG_DI,
    BX_NO_INDEX,
    BX_NO_INDEX,
    BX_NO_INDEX,
    BX_NO_INDEX,
];

const DS: u8 = BxSegregs::Ds as u8;
const SS: u8 = BxSegregs::Ss as u8;

// Default segment for 16-bit addressing, mod=00
// Matching Bochs sreg_mod00_rm16 in fetchdecode32.cc:669-678
const SREG_MOD00_RM16: [u8; 8] = [DS, DS, SS, SS, DS, DS, DS, DS];

// Default segment for 16-bit addressing, mod=01 or mod=10
// Matching Bochs sreg_mod01or10_rm16 in fetchdecode32.cc:680-689
const SREG_MOD01OR10_RM16: [u8; 8] = [DS, DS, SS, SS, DS, DS, SS, DS];

// Default segment for 32-bit addressing, mod=00
// Matching Bochs sreg_mod0_base32 in fetchdecode32.cc:692-701
const SREG_MOD0_BASE32: [u8; 8] = [DS, DS, DS, DS, SS, DS, DS, DS];

// Default segment for 32-bit addressing, mod=01 or mod=10
// Matching Bochs sreg_mod1or2_base32 in fetchdecode32.cc:703-712
const SREG_MOD1OR2_BASE32: [u8; 8] = [DS, DS, DS, DS, SS, SS, DS, DS];

/// In-place 32-bit/16-bit decoder — fills an existing [`Instruction`] slot.
///
/// Avoids creating a temporary and copying, matching
/// Bochs `fetchDecode32(BxInstruction_c *i, ...)`.
/// Prefer this over [`fetch_decode32`] in the icache miss handler.
pub const fn fetch_decode32_inplace(
    bytes: &[u8],
    is_32: bool,
    instr: &mut Instruction,
) -> DecodeResult<()> {
    *instr = Instruction {
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

    // Initialize metainfo1: os32 and as32 based on mode
    let mut metainfo1_bits: u8 = if is_32 {
        MetaInfoFlags::Os32.bits() | MetaInfoFlags::As32.bits()
    } else {
        0
    };

    let mut sse_prefix: u8 = SsePrefix::PrefixNone as u8;
    let mut seg_override: u8 = 7; // 7 = none
    let mut os_32 = is_32;
    let mut as_32 = is_32;

    // === Phase 1: Parse legacy prefixes ===
    while pos < max_len {
        let b = bytes[pos];
        match b {
            // Segment overrides
            0x26 => seg_override = 0, // ES
            0x2E => seg_override = 1, // CS
            0x36 => seg_override = 2, // SS
            0x3E => seg_override = 3, // DS
            0x64 => seg_override = 4, // FS
            0x65 => seg_override = 5, // GS

            // Operand size override
            0x66 => {
                os_32 = !is_32;
                if sse_prefix == SsePrefix::PrefixNone as u8 {
                    sse_prefix = SsePrefix::Prefix66 as u8;
                }
                if os_32 {
                    metainfo1_bits |= MetaInfoFlags::Os32.bits();
                } else {
                    metainfo1_bits &= !MetaInfoFlags::Os32.bits();
                }
            }

            // Address size override
            0x67 => {
                as_32 = !is_32;
                if as_32 {
                    metainfo1_bits |= MetaInfoFlags::As32.bits();
                } else {
                    metainfo1_bits &= !MetaInfoFlags::As32.bits();
                }
            }

            // LOCK prefix
            0xF0 => {
                metainfo1_bits = (metainfo1_bits & 0x3F) | (1 << 6);
            }

            // REPNE/REPNZ
            0xF2 => {
                metainfo1_bits = (metainfo1_bits & 0x3F) | (2 << 6);
                sse_prefix = SsePrefix::PrefixF2 as u8;
            }

            // REP/REPE/REPZ
            0xF3 => {
                metainfo1_bits = (metainfo1_bits & 0x3F) | (3 << 6);
                sse_prefix = SsePrefix::PrefixF3 as u8;
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
        instr.operands.segment = seg_override;
    } else {
        instr.operands.segment = BxSegregs::Ds as u8;
    }

    // === Phase 2: Parse opcode ===
    let mut b1 = bytes[pos] as u32;
    pos += 1;

    // Check for VEX/EVEX/XOP prefixes in 32-bit mode
    if b1 == 0xC4 || b1 == 0xC5 {
        // VEX prefix - check if it's actually VEX (mod=11)
        if pos < max_len && (bytes[pos] & 0xC0) == 0xC0 {
            // This is VEX, not LES/LDS
            return Err(DecodeError::Decoder(
                BxDecodeError::BxIllegalVexXopOpcodeMap,
            )); // VEX not fully supported in const
        }
    }

    // Note: 0x62 is EVEX prefix in 64-bit mode, but BOUND instruction in 32/16-bit mode.
    // In 32-bit mode, we need to check if it's actually EVEX or BOUND:
    // - EVEX requires P0 bit 3 = 0 AND P1 bit 2 (EVEX.U) = 1
    // - If these conditions aren't met, it's BOUND instruction
    // For now, we don't support EVEX in 32-bit mode, so 0x62 is always BOUND.
    // The BOUND instruction requires ModRM, which will be handled below.

    if b1 == 0x8F {
        // XOP prefix - check if it's actually XOP (Bochs: (modrm & 0xC8) == 0xC8)
        // Must check mod=11 AND map>=8 to distinguish from POP [mem] (mod!=11)
        if pos < max_len && (bytes[pos] & 0xC8) == 0xC8 {
            return Err(DecodeError::Decoder(
                BxDecodeError::BxIllegalVexXopOpcodeMap,
            )); // XOP not fully supported in const
        }
    }

    // Two-byte escape (0F xx)
    let mut opcode_map: u8 = 0;
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
            b1 = 0x10F;
        } else {
            b1 = 0x100 | (b2 as u32);
            opcode_map = 1;
        }
    }

    // === Phase 3: Parse ModRM if needed ===
    let needs_modrm = opcode_needs_modrm_32(b1, opcode_map);

    let mut nnn: u32 = (b1 >> 3) & 0x7;
    let mut rm: u32 = b1 & 0x7;
    let mut modrm_byte: u8 = 0; // full modrm byte, used for x87 FPU escape

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

        // MOV CR/DR (0F 20-26) always treat as register form regardless of mod field.
        // Bochs uses decoder_creg32 for 0F 20-24,26 (0F 25 is decoder_ud32).
        // Including 0F 25 in the range is harmless since it hits UD anyway.
        let force_modc0 = opcode_map == 1 && matches!(b1 & 0xFF, 0x20..=0x26);

        if mod_field == 3 || force_modc0 {
            // Register mode (or forced register for MOV CR/DR)
            metainfo1_bits |= MetaInfoFlags::ModC0.bits();
        } else {
            // Memory mode - depends on address size
            if as_32 {
                // 32-bit addressing
                let use_sib = rm == 4;

                if use_sib {
                    if pos >= max_len {
                        return Err(DecodeError::SibBufferUnderflow);
                    }

                    let sib = bytes[pos];
                    pos += 1;

                    let scale = (sib >> 6) & 0x3;
                    let index = (sib >> 3) & 0x7;
                    let base = sib & 0x7;

                    instr.operands.scale = scale;
                    instr.operands.index = index;
                    instr.operands.base = base;

                    // Displacement for SIB with base=5 and mod=0
                    if mod_field == 0 && base == 5 {
                        if pos + 4 > max_len {
                            return Err(DecodeError::DisplacementBufferUnderflow);
                        }
                        let disp = read_u32_le(bytes, pos);
                        pos += 4;
                        instr.displacement = disp;
                        instr.operands.base = BX_NIL_REGISTER;
                    }
                } else {
                    instr.operands.base = rm as u8;
                    instr.operands.index = BX_NO_INDEX;

                    // [disp32] when mod=0, rm=5
                    if mod_field == 0 && rm == 5 {
                        if pos + 4 > max_len {
                            return Err(DecodeError::DisplacementBufferUnderflow);
                        }
                        let disp = read_u32_le(bytes, pos);
                        pos += 4;
                        instr.displacement = disp;
                        instr.operands.base = BX_NIL_REGISTER;
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
            } else {
                // 16-bit addressing - no SIB
                instr.operands.base = RESOLVE16_BASE_REG[rm as usize];
                instr.operands.index = RESOLVE16_INDEX_REG[rm as usize];
                instr.operands.scale = 0;

                // [disp16] when mod=0, rm=6
                if mod_field == 0 && rm == 6 {
                    if pos + 2 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u16_le(bytes, pos);
                    pos += 2;
                    // Bochs sign-extends disp16: (Bit32s)(Bit16s) FetchWORD(iptr)
                    instr.displacement = disp as i16 as i32 as u32;
                    instr.operands.base = 19; // BX_NIL_REGISTER
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
                    // disp16 — Bochs sign-extends: (Bit32s)(Bit16s) FetchWORD(iptr)
                    if pos + 2 > max_len {
                        return Err(DecodeError::DisplacementBufferUnderflow);
                    }
                    let disp = read_u16_le(bytes, pos);
                    pos += 2;
                    instr.displacement = disp as i16 as i32 as u32;
                }
            }
        }

        // Assign default segment register based on addressing mode
        // (only if no explicit segment override prefix was used)
        // Matching Bochs fetchdecode32.cc line 2009-2010:
        //   if (! BX_NULL_SEG_REG(seg_override)) i->setSeg(seg_override);
        // But in Bochs, the default seg is set in decode_modrm functions
        // using sreg_mod00_rm16, sreg_mod01or10_rm16, etc.
        if seg_override >= 7 && mod_field != 3 {
            // No explicit prefix override - set based on addressing mode
            if !as_32 {
                // 16-bit addressing mode
                let default_seg = if mod_field == 0 {
                    SREG_MOD00_RM16[rm as usize]
                } else {
                    SREG_MOD01OR10_RM16[rm as usize]
                };
                instr.operands.segment = default_seg;
            } else {
                // 32-bit addressing mode
                let base = if rm == 4 {
                    instr.operands.base
                } else {
                    rm as u8
                };
                let default_seg = if mod_field == 0 {
                    SREG_MOD0_BASE32[base as usize & 7]
                } else {
                    SREG_MOD1OR2_BASE32[base as usize & 7]
                };
                instr.operands.segment = default_seg;
            }
        }
    } else {
        // No ModRM - instruction uses register encoded in opcode (low 3 bits = rm)
        metainfo1_bits |= MetaInfoFlags::ModC0.bits();
    }

    // Store register fields
    // For ModRM instructions: DST=nnn (reg field), SRC1=rm (r/m field)
    // EXCEPT for Group opcodes where nnn is the opcode extension, not an operand
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
                | 0x82
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
            || matches!(b1, 0x113 | 0x117 | 0x12B | 0x17F | 0x1E7)
            // 0x17E (0F 7E): Ed,Gd for no-prefix (MOVD Ed,Pq) and 66 (MOVD Ed,Vd),
            // but NOT for F3 prefix (MOVQ Vq,Wq is a LOAD: nnn=dst, rm=src)
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

    // === Phase 4: Parse immediate and moffs (direct memory offset) ===
    // Pass nnn to distinguish Group 3a/3b variants (TEST vs NOT/NEG/etc)
    let imm_size = get_immediate_size_32(b1, opcode_map, os_32, as_32, nnn);

    if imm_size > 0 {
        if pos + (imm_size as usize) > max_len {
            return Err(DecodeError::ImmediateBufferUnderflow);
        }

        match imm_size {
            1 => {
                let byte_val = bytes[pos];
                // Sign-extend byte immediates that are used as 32-bit values via id():
                // - Branch opcodes (0x70-0x7F, 0xE0-0xE3, 0xEB): relative displacements
                // - 0x83 (Group 1 EdsIb): sign-extended imm8 to operand-size per Intel spec;
                //   dispatchers route *EdsIb opcodes to *EdId handlers that read id()
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
                // ENTER: Iw + Ib
                instr.immediate = read_u16_le(bytes, pos) as u32;
                instr.displacement = bytes[pos + 2] as u32;
                pos += 3;
            }
            4 => {
                // Check if this is a far pointer (0x9A CALL FAR, 0xEA JMP FAR)
                let is_far_pointer = matches!(b1, 0x9A | 0xEA);
                if is_far_pointer {
                    // Far pointer in 16-bit mode: Iw (offset) + Iw (segment)
                    instr.immediate = read_u16_le(bytes, pos) as u32;
                    instr.displacement = read_u16_le(bytes, pos + 2) as u32;
                } else {
                    // Regular 4-byte immediate
                    instr.immediate = read_u32_le(bytes, pos);
                }
                pos += 4;
            }
            6 => {
                // Far pointer in 32-bit mode: Id (offset) + Iw (segment)
                instr.immediate = read_u32_le(bytes, pos);
                instr.displacement = read_u16_le(bytes, pos + 4) as u32;
                pos += 6;
            }
            _ => {}
        }
    }

    // Finalize instruction
    instr.length = pos as u8;
    instr.flags = MetaInfoFlags::from_bits_retain(metainfo1_bits);

    // Build decmask for opcode lookup
    // Match C++ implementation: decmask uses i->osize() and i->asize() which return actual values
    let mod_c0 = (metainfo1_bits & MetaInfoFlags::ModC0.bits()) != 0;
    // Extract osize and asize from metainfo1 bits (same as osize() and asize() methods)
    // osize = (bits >> 2) & 0x3, asize = bits & 0x3
    let osize_val = ((metainfo1_bits >> 2) & 0x3) as u32;
    let asize_val = (metainfo1_bits & 0x3) as u32;

    // Match C++ implementation exactly:
    // - decoder32 (no ModRM): decmask includes osize, asize, sse_prefix, MODC0, and SRC_EQ_DST_OFFSET if nnn==rm
    // - decoder32_modrm: decmask includes osize, asize, sse_prefix, MODC0, nnn, rm, and SRC_EQ_DST_OFFSET if mod_c0 && nnn==rm
    // No IS32_OFFSET bit in 32-bit mode decmask
    let lock_rep_value = (metainfo1_bits >> 6) & 0x3;
    let decmask: u32 = (osize_val << OS32_OFFSET)
        | (asize_val << AS32_OFFSET)
        | ((sse_prefix as u32) << SSE_PREFIX_OFFSET)
        | (if lock_rep_value == 1 { 1 } else { 0 } << LOCK_PREFIX_OFFSET)
        | (if mod_c0 { 1 } else { 0 } << MODC0_OFFSET)
        | if needs_modrm {
            (rm << RRR_OFFSET) | (nnn << NNN_OFFSET)
        } else {
            0
        }
        | if mod_c0 && nnn == rm {
            1 << SRC_EQ_DST_OFFSET
        } else {
            0
        };

    // Look up opcode from tables
    if opcode_map == 0 && (b1 >= 0xD8 && b1 <= 0xDF) {
        // x87 FPU escape opcodes — use dedicated FPU opcode tables
        // Matching Bochs decoder32_fp_escape() in fetchdecode32.cc
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
            nnn as usize
        };
        instr.opcode = fpu_table[fpu_index];
        // Store foo: (modrm | (escape_byte << 8)) & 0x7FF — for x87 FPU handler context
        // Can't call set_foo() in const fn, so set id directly (foo is in lower 16 bits of id)
        let foo_val = ((modrm_byte as u16) | ((b1 as u16) << 8)) & 0x7FF;
        instr.immediate = foo_val as u32;
    } else if opcode_map == 4 {
        // 3DNow! instruction: use suffix to look up opcode directly
        instr.opcode = BX3_DNOW_OPCODE[dnow_suffix as usize];
    } else if opcode_map == 0 && b1 == 0x90 {
        // Special NOP/PAUSE handling (Bochs decoder32_nop)
        if sse_prefix == SsePrefix::PrefixF3 as u8 {
            // F3 prefix → PAUSE
            instr.opcode = Opcode::Pause;
        } else {
            // Bare 0x90 → NOP
            instr.opcode = Opcode::Nop;
        }
    } else {
        instr.opcode = lookup_opcode_32(b1, opcode_map, decmask, nnn);
    }

    // Check if opcode lookup failed
    if matches!(instr.opcode, Opcode::IaError) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    // Post-decode LOCK validation (Bochs fetchdecode32.cc:2014-2034)
    // LOCK prefix on register operand (modC0) is always invalid → #UD
    let has_lock = (metainfo1_bits >> 6) & 0x3 == 1;
    let mod_c0 = (metainfo1_bits & MetaInfoFlags::ModC0.bits()) != 0;
    if has_lock && mod_c0 {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    Ok(())
}

/// Original return-value API. Uses fetch_decode32_inplace internally.
/// For runtime hot-path icache misses, prefer fetch_decode32_inplace to eliminate the copy.
pub const fn fetch_decode32(bytes: &[u8], is_32: bool) -> DecodeResult<Instruction> {
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
    // Use match instead of ? — const_try is not yet stable
    match fetch_decode32_inplace(bytes, is_32, &mut instr) {
        Ok(()) => Ok(instr),
        Err(e) => Err(e),
    }
}

/// Get opcode table and look up opcode for 32-bit mode
const fn lookup_opcode_32(b1: u32, opcode_map: u8, decmask: u32, _nnn: u32) -> Opcode {
    if opcode_map == 0 {
        // One-byte opcodes
        let table = get_opcode_table_32(b1 as u8);
        if table.is_empty() {
            return Opcode::IaError;
        }
        find_opcode_in_table(table, decmask)
    } else if opcode_map == 1 {
        // Two-byte opcodes (0F xx)
        let table = get_opcode_table_0f_32((b1 & 0xFF) as u8);
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

/// Get opcode table for one-byte opcode in 32-bit mode
const fn get_opcode_table_32(b1: u8) -> &'static [u64] {
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
        0x62 => &BxOpcodeTable62, // BOUND instruction
        0x63 => &BxOpcodeTable63_32,
        0x68 => &BxOpcodeTable68,
        0x69 => &BxOpcodeTable69,
        0x6A => &BxOpcodeTable6A,
        0x6B => &BxOpcodeTable6B,
        0x6C => &BxOpcodeTable6C,
        0x6D => &BxOpcodeTable6D,
        0x6E => &BxOpcodeTable6E,
        0x6F => &BxOpcodeTable6F,
        0x70 => &BxOpcodeTable70_32,
        0x71 => &BxOpcodeTable71_32,
        0x72 => &BxOpcodeTable72_32,
        0x73 => &BxOpcodeTable73_32,
        0x74 => &BxOpcodeTable74_32,
        0x75 => &BxOpcodeTable75_32,
        0x76 => &BxOpcodeTable76_32,
        0x77 => &BxOpcodeTable77_32,
        0x78 => &BxOpcodeTable78_32,
        0x79 => &BxOpcodeTable79_32,
        0x7A => &BxOpcodeTable7A_32,
        0x7B => &BxOpcodeTable7B_32,
        0x7C => &BxOpcodeTable7C_32,
        0x7D => &BxOpcodeTable7D_32,
        0x7E => &BxOpcodeTable7E_32,
        0x7F => &BxOpcodeTable7F_32,
        0x80 => &BxOpcodeTable80,
        0x81 => &BxOpcodeTable81,
        0x82 => &BxOpcodeTable80, // opcode 0x82 is copy of 0x80 (Bochs fetchdecode32.cc:247)
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
        0x90..=0x97 => &BxOpcodeTable90x97,
        0x98 => &BxOpcodeTable98,
        0x99 => &BxOpcodeTable99,
        0x9A => &BxOpcodeTable9A,
        0x9B => &BxOpcodeTable9B,
        0x9C => &BxOpcodeTable9C,
        0x9D => &BxOpcodeTable9D,
        0x9E => &BxOpcodeTable9E_32,
        0x9F => &BxOpcodeTable9F_32,
        0xA0 => &BxOpcodeTableA0_32,
        0xA1 => &BxOpcodeTableA1_32,
        0xA2 => &BxOpcodeTableA2_32,
        0xA3 => &BxOpcodeTableA3_32,
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
        0xC2 => &BxOpcodeTableC2_32,
        0xC3 => &BxOpcodeTableC3_32,
        0xC4 => &BxOpcodeTableC4_32,
        0xC5 => &BxOpcodeTableC5_32,
        0xC6 => &BxOpcodeTableC6,
        0xC7 => &BxOpcodeTableC7,
        0xC8 => &BxOpcodeTableC8_32,
        0xC9 => &BxOpcodeTableC9_32,
        0xCA => &BxOpcodeTableCA,
        0xCB => &BxOpcodeTableCB,
        0xCC => &BxOpcodeTableCC,
        0xCD => &BxOpcodeTableCD,
        0xCE => &BxOpcodeTableCE,
        0xCF => &BxOpcodeTableCF_32,
        0xD0 => &BxOpcodeTableD0,
        0xD1 => &BxOpcodeTableD1,
        0xD2 => &BxOpcodeTableD2,
        0xD3 => &BxOpcodeTableD3,
        0xD4 => &BxOpcodeTableD4,
        0xD5 => &BxOpcodeTableD5,
        0xD6 => &BxOpcodeTableD6,
        0xD7 => &BxOpcodeTableD7,
        0xE0 => &BxOpcodeTableE0_32,
        0xE1 => &BxOpcodeTableE1_32,
        0xE2 => &BxOpcodeTableE2_32,
        0xE3 => &BxOpcodeTableE3_32,
        0xE4 => &BxOpcodeTableE4,
        0xE5 => &BxOpcodeTableE5,
        0xE6 => &BxOpcodeTableE6,
        0xE7 => &BxOpcodeTableE7,
        0xE8 => &BxOpcodeTableE8_32,
        0xE9 => &BxOpcodeTableE9_32,
        0xEA => &BxOpcodeTableEA_32,
        0xEB => &BxOpcodeTableEB_32,
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

/// Get opcode table for two-byte opcode (0F xx) in 32-bit mode
const fn get_opcode_table_0f_32(b2: u8) -> &'static [u64] {
    match b2 {
        0x00 => &BxOpcodeTable0F00,
        0x01 => &BxOpcodeTable0F01,
        0x02 => &BxOpcodeTable0F02,
        0x03 => &BxOpcodeTable0F03,
        0x05 => &BxOpcodeTable0F05_32,
        0x06 => &BxOpcodeTable0F06,
        0x07 => &BxOpcodeTable0F07_32,
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
        // 0F 19..1D and 0F 1F: multi-byte NOPs (Bochs: BxOpcodeTableMultiByteNOP)
        // These require ModRM to determine instruction length but execute as NOP
        0x19..=0x1D | 0x1F => &BxOpcodeTableMultiByteNOP,
        0x1E => &BxOpcodeTable0F1E,
        0x20 => &BxOpcodeTable0F20_32,
        0x21 => &BxOpcodeTable0F21_32,
        0x22 => &BxOpcodeTable0F22_32,
        0x23 => &BxOpcodeTable0F23_32,
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
        0x80 => &BxOpcodeTable0F80_32,
        0x81 => &BxOpcodeTable0F81_32,
        0x82 => &BxOpcodeTable0F82_32,
        0x83 => &BxOpcodeTable0F83_32,
        0x84 => &BxOpcodeTable0F84_32,
        0x85 => &BxOpcodeTable0F85_32,
        0x86 => &BxOpcodeTable0F86_32,
        0x87 => &BxOpcodeTable0F87_32,
        0x88 => &BxOpcodeTable0F88_32,
        0x89 => &BxOpcodeTable0F89_32,
        0x8A => &BxOpcodeTable0F8A_32,
        0x8B => &BxOpcodeTable0F8B_32,
        0x8C => &BxOpcodeTable0F8C_32,
        0x8D => &BxOpcodeTable0F8D_32,
        0x8E => &BxOpcodeTable0F8E_32,
        0x8F => &BxOpcodeTable0F8F_32,
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

/// Check if opcode needs ModRM byte (32-bit mode)
const fn opcode_needs_modrm_32(b1: u32, map: u8) -> bool {
    if map == 0 {
        let opcode = b1 as u8;
        !matches!(opcode,
            0x04 | 0x05 | 0x0C | 0x0D | 0x14 | 0x15 | 0x1C | 0x1D |
            0x24 | 0x25 | 0x2C | 0x2D | 0x34 | 0x35 | 0x3C | 0x3D |
            0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F |
            0x27 | 0x2F | 0x37 | 0x3F |
            0x40..=0x5F |
            0x60..=0x61 | 0x68 | 0x6A | 0x6C..=0x6F |  // 0x62 (BOUND) needs ModRM, not in this list
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
        true
    }
}

/// Get immediate size for opcode (32-bit mode)
/// Also handles moffs (direct memory offset) for opcodes A0-A3
const fn get_immediate_size_32(b1: u32, map: u8, os_32: bool, as_32: bool, nnn: u32) -> u8 {
    if map == 0 {
        let opcode = b1 as u8;
        match opcode {
            // moffs (direct memory offset) - depends on ADDRESS size, not operand size
            // A0 = MOV AL, [moffs8]
            // A1 = MOV AX/EAX, [moffs]
            // A2 = MOV [moffs8], AL
            // A3 = MOV [moffs], AX/EAX
            0xA0..=0xA3 => {
                if as_32 {
                    4 // 32-bit address = 4-byte offset
                } else {
                    2 // 16-bit address = 2-byte offset
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
            // Based on Bochs cpu/decoder/fetchdecode32.cc:888-1077 (fetchImmediate)
            // and opcodes table entries for Group 3a
            0xF6
                if (nnn == 0 || nnn == 1) => {
                    1 // TEST r/m8, imm8
                }

            // Group 3b (F7): TEST (nnn=0,1) has Iv, others have no immediate
            0xF7
                if (nnn == 0 || nnn == 1) => {
                    if os_32 {
                        4 // TEST r/m32, imm32
                    } else {
                        2 // TEST r/m16, imm16
                    }
                }

            // Iw
            0xC2 | 0xCA => 2,

            // ENTER: Iw + Ib = 3 bytes
            0xC8 => 3,

            // Iv (operand-size dependent)
            0x05
            | 0x0D
            | 0x15
            | 0x1D
            | 0x25
            | 0x2D
            | 0x35
            | 0x3D
            | 0x68
            | 0x69
            | 0xA9
            | 0xE8
            | 0xE9
            | 0x81
            | 0xC7
            | 0xB8..=0xBF => {
                if os_32 {
                    4
                } else {
                    2
                }
            }

            // Far pointer (Ap): offset + segment
            // 16-bit: Iw + Iw = 4 bytes (2-byte offset + 2-byte segment)
            // 32-bit: Id + Iw = 6 bytes (4-byte offset + 2-byte segment)
            0x9A | 0xEA => {
                if os_32 {
                    6 // 32-bit mode: 4-byte offset + 2-byte segment
                } else {
                    4 // 16-bit mode: 2-byte offset + 2-byte segment
                }
            }

            _ => 0,
        }
    } else if map == 1 {
        let opcode = (b1 & 0xFF) as u8;
        match opcode {
            // Jcc rel32/rel16
            0x80..=0x8F => {
                if os_32 {
                    4
                } else {
                    2
                }
            }
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
    use super::*;

    #[test]
    fn test_nop() {
        // 0x90 is NOP (Bochs decoder32_nop returns NOP for bare 0x90)
        let i = fetch_decode32(&[0x90], true).unwrap();
        assert_eq!(i.ilen(), 1);
        assert_eq!(i.get_ia_opcode(), Opcode::Nop);
    }

    #[test]
    fn test_ret() {
        let i = fetch_decode32(&[0xC3], true).unwrap();
        assert_eq!(i.ilen(), 1);
    }

    #[test]
    fn test_inc_eax() {
        let i = fetch_decode32(&[0x40], true).unwrap(); // INC EAX
        assert_eq!(i.ilen(), 1);
    }

    #[test]
    fn test_push_pop() {
        let i = fetch_decode32(&[0x50], true).unwrap(); // PUSH EAX
        assert_eq!(i.ilen(), 1);

        let i = fetch_decode32(&[0x5B], true).unwrap(); // POP EBX
        assert_eq!(i.ilen(), 1);
    }

    #[test]
    fn test_modrm_reg() {
        let i = fetch_decode32(&[0x89, 0xD8], true).unwrap(); // MOV EAX, EBX
        assert_eq!(i.ilen(), 2);
        assert!(i.mod_c0());
    }

    #[test]
    fn test_modrm_mem() {
        let i = fetch_decode32(&[0x8B, 0x03], true).unwrap(); // MOV EAX, [EBX]
        assert_eq!(i.ilen(), 2);
        assert!(!i.mod_c0());
    }

    #[test]
    fn test_sib() {
        let i = fetch_decode32(&[0x8B, 0x04, 0x8B], true).unwrap(); // MOV EAX, [EBX+ECX*4]
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.sib_scale(), 2); // *4
    }

    #[test]
    fn test_16bit_mode() {
        let i = fetch_decode32(&[0x8B, 0x00], false).unwrap(); // MOV AX, [BX+SI]
        assert_eq!(i.ilen(), 2);
    }

    #[test]
    fn test_16bit_disp() {
        let i = fetch_decode32(&[0x8B, 0x06, 0x34, 0x12], false).unwrap(); // MOV AX, [0x1234]
        assert_eq!(i.ilen(), 4);
        assert_eq!(i.displacement, 0x1234);
    }

    #[test]
    fn test_os_override_32() {
        let i = fetch_decode32(&[0x66, 0xB8, 0x01, 0x02], true).unwrap();
        assert_eq!(i.ilen(), 4);
        assert_eq!(i.immediate, 0x0201);
    }

    #[test]
    fn test_os_override_16() {
        let i = fetch_decode32(&[0x66, 0xB8, 0x01, 0x02, 0x03, 0x04], false).unwrap();
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.immediate, 0x04030201);
    }

    #[test]
    fn test_disp8() {
        let i = fetch_decode32(&[0x8B, 0x43, 0x10], true).unwrap(); // MOV EAX, [EBX+0x10]
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.displacement, 0x10);
    }

    #[test]
    fn test_disp32() {
        let i = fetch_decode32(&[0x8B, 0x83, 0x78, 0x56, 0x34, 0x12], true).unwrap();
        assert_eq!(i.ilen(), 6);
        assert_eq!(i.displacement, 0x12345678);
    }

    #[test]
    fn test_imm32() {
        let i = fetch_decode32(&[0x68, 0x78, 0x56, 0x34, 0x12], true).unwrap();
        assert_eq!(i.ilen(), 5);
        assert_eq!(i.immediate, 0x12345678);
    }

    #[test]
    fn test_enter() {
        let i = fetch_decode32(&[0xC8, 0x10, 0x00, 0x01], true).unwrap(); // ENTER 0x10, 1
        assert_eq!(i.ilen(), 4);
        assert_eq!(i.immediate, 0x10);
        assert_eq!(i.displacement, 1);
    }

    #[test]
    fn test_0f_opcode() {
        let i = fetch_decode32(&[0x0F, 0xA2], true).unwrap(); // CPUID
        assert_eq!(i.ilen(), 2);
    }

    #[test]
    fn test_lock() {
        let i = fetch_decode32(&[0xF0, 0x87, 0x03], true).unwrap(); // LOCK XCHG
        assert_eq!(i.ilen(), 3);
        assert!(i.get_lock());
    }

    #[test]
    fn test_segment() {
        let i = fetch_decode32(&[0x2E, 0x8B, 0x00], true).unwrap(); // CS: prefix
        assert_eq!(i.ilen(), 3);
        assert_eq!(i.seg(), 1); // CS
    }

    #[test]
    fn test_empty() {
        let result = fetch_decode32(&[], true);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DecodeError::BufferUnderflow));
    }

    #[test]
    fn test_0f38() {
        let i = fetch_decode32(&[0x66, 0x0F, 0x38, 0x00, 0xC1], true).unwrap(); // PSHUFB
        assert_eq!(i.ilen(), 5);
    }

    #[test]
    fn test_out_instruction() {
        let i = fetch_decode32(&[0xE6, 0x0d], false).unwrap(); // OUT 0x0D, AL
        assert_eq!(i.ilen(), 2);
        assert_eq!(i.get_ia_opcode(), Opcode::OutIbAl);
        assert_eq!(i.immediate, 0x0d);
        assert_eq!(i.displacement, 0x00);
    }

    /// Test that valid segment registers (0-5) decode successfully for MOV Ew,Sw and MOV Sw,Ew
    #[test]
    fn test_mov_segment_valid() {
        // Test opcodes 0x8C (MOV r/m16, Sreg) and 0x8E (MOV Sreg, r/m16) with nnn=0 through nnn=5
        for seg in 0..=5 {
            let modrm = 0xC0 | (seg << 3); // MOD=11, REG=seg, R/M=0 (AX)

            // 0x8C: MOV r/m16, Sreg
            let bytes = vec![0x8C, modrm];
            let result = fetch_decode32(&bytes, true);
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
            let result = fetch_decode32(&bytes, true);
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

    /// Test that invalid segment registers (6-7) are rejected with InvalidSegmentRegister error
    #[test]
    fn test_mov_segment_invalid() {
        // Test opcodes 0x8C and 0x8E with nnn=6 and nnn=7
        for seg in 6..=7 {
            let modrm = 0xC0 | (seg << 3); // MOD=11, REG=seg, R/M=0

            // 0x8C: MOV r/m16, Sreg - should fail with InvalidSegmentRegister
            let bytes = vec![0x8C, modrm];
            let result = fetch_decode32(&bytes, true);
            assert!(
                matches!(result, Err(DecodeError::InvalidSegmentRegister { index, opcode: 0x8C }) if index == seg),
                "Should reject invalid segment register {} for opcode 0x8C, got: {:?}",
                seg,
                result
            );

            // 0x8E: MOV Sreg, r/m16 - should fail with InvalidSegmentRegister
            let bytes = vec![0x8E, modrm];
            let result = fetch_decode32(&bytes, true);
            assert!(
                matches!(result, Err(DecodeError::InvalidSegmentRegister { index, opcode: 0x8E }) if index == seg),
                "Should reject invalid segment register {} for opcode 0x8E, got: {:?}",
                seg,
                result
            );
        }
    }

    /// Test that 0x83 (Group 1 EdsIb) sign-extends the immediate byte
    #[test]
    fn test_0x83_sign_extension() {
        // 83 C3 FD = ADD EBX, -3 (sign-extended 0xFD to 0xFFFFFFFD)
        let bytes = vec![0x83, 0xC3, 0xFD];
        let instr = fetch_decode32(&bytes, true).unwrap();
        assert_eq!(
            instr.id(),
            0xFFFFFFFD,
            "0x83 imm8 0xFD should be sign-extended to 0xFFFFFFFD, got {:#x}",
            instr.id()
        );

        // 83 C3 08 = ADD EBX, 8 (positive stays same)
        let bytes = vec![0x83, 0xC3, 0x08];
        let instr = fetch_decode32(&bytes, true).unwrap();
        assert_eq!(
            instr.id(),
            0x00000008,
            "0x83 imm8 0x08 should stay 0x00000008, got {:#x}",
            instr.id()
        );

        // 83 FB FF = CMP EBX, -1 (sign-extended)
        let bytes = vec![0x83, 0xFB, 0xFF];
        let instr = fetch_decode32(&bytes, true).unwrap();
        assert_eq!(
            instr.id(),
            0xFFFFFFFF,
            "0x83 imm8 0xFF should be sign-extended to 0xFFFFFFFF, got {:#x}",
            instr.id()
        );
    }
}
