//! 64-bit instruction decoder
//!
//! This module implements x86-64 instruction decoding, including:
//! - REX prefix handling (0x40-0x4F)
//! - Extended registers (R8-R15)
//! - RIP-relative addressing
//! - 64-bit immediates
//!
//! Based on Bochs cpp/cpu/decoder/fetchdecode64.cc

use super::{
    fetchdecode_generated::{BxDecodeError, *},
    fetchdecode_opmap::*,
    fetchdecode_opmap_0f38::BxOpcodeTable0F38,
    fetchdecode_opmap_0f3a::BxOpcodeTable0F3A,
    instr_generated::BxInstructionGenerated,
    instr::MetaInfoFlags,
    ia_opcodes::Opcode,
    BxSegregs, DecodeResult,
    fetchdecode32,
    fetchdecode::SsePrefix,
};

use crate::cpu::decoder::fetchdecode::*;

/// REX prefix structure
/// 
/// REX prefix (0x40-0x4F) extends register fields:
/// - W: Operand size (0=default, 1=64-bit)
/// - R: Extends ModRM reg field (bit 3)
/// - X: Extends SIB index field (bit 3)
/// - B: Extends ModRM r/m or SIB base field (bit 3)
#[derive(Debug, Clone, Copy, Default)]
struct RexPrefix {
    /// REX.W bit (bit 3)
    w: bool,
    /// REX.R bit (bit 2)
    r: bool,
    /// REX.X bit (bit 1)
    x: bool,
    /// REX.B bit (bit 0)
    b: bool,
}

impl RexPrefix {
    /// Parse REX prefix from byte (0x40-0x4F)
    fn from_byte(byte: u8) -> Option<Self> {
        if byte >= 0x40 && byte <= 0x4F {
            Some(Self {
                w: (byte & 0x8) != 0,
                r: (byte & 0x4) != 0,
                x: (byte & 0x2) != 0,
                b: (byte & 0x1) != 0,
            })
        } else {
            None
        }
    }
    
    /// Get raw REX prefix value (0x40-0x4F)
    fn to_byte(&self) -> u8 {
        0x40 | (u8::from(self.w) << 3) | (u8::from(self.r) << 2) | (u8::from(self.x) << 1) | u8::from(self.b)
    }
    
    /// Check if REX prefix is present (any REX byte)
    fn is_present(&self) -> bool {
        self.w || self.r || self.x || self.b
    }
    
    /// Get REX.R value (for extending ModRM reg field)
    fn rex_r(&self) -> u32 {
        u32::from(self.r) << 3
    }
    
    /// Get REX.X value (for extending SIB index field)
    fn rex_x(&self) -> u32 {
        u32::from(self.x) << 3
    }
    
    /// Get REX.B value (for extending ModRM r/m or SIB base field)
    fn rex_b(&self) -> u32 {
        u32::from(self.b) << 3
    }
}

/// Segment register encoding for 64-bit mode
/// 
/// In 64-bit mode, CS, DS, ES, and SS segment overrides are ignored.
/// Only FS: (0x64) and GS: (0x65) are valid.
const SREG_MOD0_BASE64: [BxSegregs; 16] = [
    BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
    BxSegregs::Ss, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
    BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
    BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
];

const SREG_MOD1OR2_BASE64: [BxSegregs; 16] = [
    BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
    BxSegregs::Ss, BxSegregs::Ss, BxSegregs::Ds, BxSegregs::Ds,
    BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
    BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds, BxSegregs::Ds,
];

/// Decode ModRM for 64-bit mode with REX prefix support
/// 
/// This function handles RIP-relative addressing and REX prefix extension
/// of register fields. Based on decodeModrm64 from the C++ implementation.
fn decode_modrm64<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    mod_field: u32,
    nnn: u32,
    rm: u32,
    rex_r: u32,
    rex_x: u32,
    rex_b: u32,
) -> DecodeResult<&'a [u8]> {
    let mut seg = BxSegregs::Ds;
    
    // Initialize displacement with zero
    i.modrm_form.displacement.set_displ32u(0);
    
    // Note: mod==11b (register mode) is handled outside
    
    let rm_extended = (rm & 0x7) | (rex_b & 0x8);
    
    if rm_extended != 4 {
        // No SIB byte
        i.set_sib_base(rm_extended.try_into().map_err(|_| BxDecodeError::U32toUsize)?);
        i.set_sib_index(4); // No index encoding by default
        
        if mod_field == 0x00 {
            // mod == 00b
            if (rm & 0x7) == 5 {
                // RIP-relative addressing (64-bit mode only)
                i.set_sib_base(super::BX_64BIT_REG_RIP.try_into().map_err(|_| BxDecodeError::U32toUsize)?);
                // Fetch 32-bit displacement
                if iptr.len() < 4 {
                    return Err(BxDecodeError::NoMoreLen.into());
                }
                let disp = fetch_dword(iptr);
                iptr = &iptr[4..];
                i.modrm_form.displacement.set_displ32u(disp);
                i.set_seg(seg);
                return Ok(iptr);
            }
            // mod==00b, rm!=4, rm!=5
            i.set_seg(seg);
            return Ok(iptr);
        }
        // mod==01b or mod==10b
        seg = SREG_MOD1OR2_BASE64[usize::try_from(rm_extended).map_err(|_| BxDecodeError::U32toUsize)?];
    } else {
        // mod!=11b, rm==4, SIB byte follows
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        
        let sib = iptr[0];
        iptr = &iptr[1..];
        
        let base = ((sib & 0x7) as u32) | rex_b;
        let index = (((sib >> 3) & 0x7) as u32) | rex_x;
        let scale = (sib >> 6) & 0x3;
        
        i.set_sib_scale(scale);
        i.set_sib_base(base.try_into().map_err(|_| BxDecodeError::U32toUsize)?);
        i.set_sib_index(index.try_into().map_err(|_| BxDecodeError::U32toUsize)?);
        
        if mod_field == 0x00 {
            // mod==00b, rm==4
            seg = SREG_MOD0_BASE64[usize::try_from(base).map_err(|_| BxDecodeError::U32toUsize)?];
            if (base & 0x7) == 5 {
                // No base register
                i.set_sib_base(super::BX_NIL_REGISTER.try_into().map_err(|_| BxDecodeError::U32toUsize)?);
                // Fetch 32-bit displacement
                if iptr.len() < 4 {
                    return Err(BxDecodeError::NoMoreLen.into());
                }
                let disp = fetch_dword(iptr);
                iptr = &iptr[4..];
                i.modrm_form.displacement.set_displ32u(disp);
                i.set_seg(seg);
                return Ok(iptr);
            }
            // mod==00b, rm==4, base!=5
            i.set_seg(seg);
            return Ok(iptr);
        }
        // mod==01b or mod==10b
        seg = SREG_MOD1OR2_BASE64[usize::try_from(base).map_err(|_| BxDecodeError::U32toUsize)?];
    }
    
    // Handle displacement based on mod field
    if mod_field == 0x40 {
        // mod == 01b: 8-bit sign-extended displacement
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let signed_byte = iptr[0] as i8;
        let disp = u32::try_from(i32::from(signed_byte)).unwrap_or(0);
        iptr = &iptr[1..];
        i.modrm_form.displacement.set_displ32u(disp);
    } else {
        // mod == 10b: 32-bit displacement
        if iptr.len() < 4 {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let disp = fetch_dword(iptr);
        iptr = &iptr[4..];
        i.modrm_form.displacement.set_displ32u(disp);
    }
    
    i.set_seg(seg);
    Ok(iptr)
}

/// Parse ModRM byte for 64-bit mode
/// 
/// Extracts mod, nnn, and rm fields, applying REX prefix extensions.
fn parse_modrm64<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    rex_r: u32,
    rex_x: u32,
    rex_b: u32,
) -> DecodeResult<(BxModrm, &'a [u8])> {
    if iptr.is_empty() {
        return Err(BxDecodeError::ModRmParseFail.into());
    }
    
    let b2 = u32::from(iptr[0]);
    iptr = &iptr[1..];
    
    let mut modrm = BxModrm::default();
    modrm.modrm = b2;
    modrm.mod_ = b2 & 0xc0;
    modrm.nnn = ((b2 >> 3) & 0x7) | rex_r;
    modrm.rm = (b2 & 0x7) | rex_b;
    
    if modrm.mod_ == 0xc0 {
        // mod == 11b (register mode)
        i.assert_mod_c0();
    } else {
        // Memory mode: decode addressing
        iptr = decode_modrm64(iptr, i, modrm.mod_, modrm.nnn, modrm.rm, rex_r, rex_x, rex_b)?;
    }
    
    Ok((modrm, iptr))
}

/// Decode 64-bit instructions with ModRM byte
/// 
/// Based on decoder64_modrm from the C++ implementation.
fn decoder64_modrm<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    rex_prefix: u8,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode32::{assign_srcs, fetch_immediate, find_opcode};
    
    let Some(opcode_table) = opcode_table else {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    };
    
    let rex = RexPrefix::from_byte(rex_prefix).unwrap_or_default();
    let rex_r = rex.rex_r();
    let rex_x = rex.rex_x();
    let rex_b = rex.rex_b();
    
    let (modrm, updated_iptr) = parse_modrm64(iptr, i, rex_r, rex_x, rex_b)?;
    iptr = updated_iptr;
    
    // Construct decmask
    let sse_prefix_raw = match sse_prefix {
        Some(prefix) => prefix as u32,
        None => 0,
    };
    
    let mut decmask = (1u32 << IS64_OFFSET) // 64-bit mode flag
        | (u32::from(i.osize()) << OS32_OFFSET)
        | (u32::from(i.asize()) << AS32_OFFSET)
        | (sse_prefix_raw << SSE_PREFIX_OFFSET)
        | if i.mod_c0() { 1 << MODC0_OFFSET } else { 0 }
        | ((modrm.nnn & 0x7) << NNN_OFFSET)
        | ((modrm.rm & 0x7) << RRR_OFFSET);
    
    if i.mod_c0() && modrm.nnn == modrm.rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }
    
    // Find opcode
    let ia_opcode = fetchdecode32::find_opcode(opcode_table, decmask)?;
    
    // Fetch immediate value if needed
    iptr = fetchdecode32::fetch_immediate(iptr, i, ia_opcode, true)?;
    
    // Assign sources
    fetchdecode32::assign_srcs(i, ia_opcode, modrm.nnn, modrm.rm)?;
    
    Ok((ia_opcode, iptr))
}

/// Decode 64-bit instructions without ModRM byte
/// 
/// Based on decoder64 from the C++ implementation.
fn decoder64<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode32::{assign_srcs, fetch_immediate, find_opcode};
    
    let Some(opcode_table) = opcode_table else {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    };
    
    let rm: u32 = b1 & 0x7;
    let nnn: u32 = (b1 >> 3) & 0x7;
    
    let sse_prefix_raw = match sse_prefix {
        Some(prefix) => prefix as u32,
        None => 0,
    };
    
    let mut decmask = (1u32 << IS64_OFFSET) // 64-bit mode flag
        | (u32::from(i.osize()) << OS32_OFFSET)
        | (u32::from(i.asize()) << AS32_OFFSET)
        | (sse_prefix_raw << SSE_PREFIX_OFFSET)
        | (1 << MODC0_OFFSET); // Register-only instructions
    
    if nnn == rm {
        decmask |= 1 << SRC_EQ_DST_OFFSET;
    }
    
    let ia_opcode = fetchdecode32::find_opcode(opcode_table, decmask)?;
    
    // Fetch immediate value if needed
    iptr = fetchdecode32::fetch_immediate(iptr, i, ia_opcode, true)?;
    
    // Assign sources (decoder64 is for register-only instructions)
    i.assert_mod_c0();
    fetchdecode32::assign_srcs(i, ia_opcode, nnn, rm)?;
    
    Ok((ia_opcode, iptr))
}

/// Decode simple 64-bit instructions
/// 
/// Based on decoder_simple64 from the C++ implementation.
/// 
/// For simple instructions with no immediate expected and no sources expected,
/// takes the first opcode from the opcode table.
fn decoder_simple64<'a>(
    _iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    i.assert_mod_c0();
    
    let Some(opcode_table) = opcode_table else {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    };
    
    // No immediate expected, no sources expected, take first opcode
    // Extract opcode from first entry: (op >> 48) & 0x7FFF
    if opcode_table.is_empty() {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }
    
    let op = opcode_table[0];
    let ia_opcode_raw = u16::try_from((op >> 48) & 0x7FFF).unwrap_or(0);
    let ia_opcode = Opcode::try_from(ia_opcode_raw)?;
    
    Ok((ia_opcode, _iptr))
}

/// Decode control register instructions (64-bit)
/// 
/// Based on decoder_creg64 from the C++ implementation.
/// 
/// MOVs with CRx and DRx always use register ops and ignore the mod field.
fn decoder_creg64<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    rex_prefix: u8,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode32;
    
    // MOVs with CRx and DRx always use register ops and ignore the mod field.
    // b1 should be 0x120-0x127 (0x120 | nnn)
    assert!((b1 & !7) == 0x120, "decoder_creg64: invalid b1 value");
    
    let rex = RexPrefix::from_byte(rex_prefix).unwrap_or_default();
    let rex_r = rex.rex_r();
    let rex_b = rex.rex_b();
    
    // opcode requires modrm byte
    if iptr.is_empty() {
        return Err(BxDecodeError::ModRmParseFail.into());
    }
    
    let b2 = u32::from(iptr[0]);
    iptr = &iptr[1..];
    
    // Parse mod-nnn-rm and related bytes
    let nnn = ((b2 >> 3) & 0x7) | rex_r;
    let rm = (b2 & 0x7) | rex_b;
    
    i.assert_mod_c0();
    
    let sse_prefix_raw = match sse_prefix {
        Some(prefix) => prefix as u32,
        None => 0,
    };
    
    let mut decmask = (1u32 << IS64_OFFSET)
        | (u32::from(i.osize()) << OS32_OFFSET)
        | (u32::from(i.asize()) << AS32_OFFSET)
        | (sse_prefix_raw << SSE_PREFIX_OFFSET)
        | (1 << MODC0_OFFSET)
        | ((nnn & 0x7) << NNN_OFFSET)
        | ((rm & 0x7) << RRR_OFFSET);
    
    let Some(opcode_table) = opcode_table else {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    };
    
    let ia_opcode = fetchdecode32::find_opcode(opcode_table, decmask)?;
    
    // Assign sources
    fetchdecode32::assign_srcs(i, ia_opcode, nnn, rm)?;
    
    Ok((ia_opcode, iptr))
}

/// Decode x87 FPU escape instructions (64-bit)
/// 
/// Based on decoder64_fp_escape from the C++ implementation.
fn decoder64_fp_escape<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode32;
    use super::fetchdecode_x87::{
        BxOpcodeInfo_FloatingPointD8, BxOpcodeInfo_FloatingPointD9,
        BxOpcodeInfo_FloatingPointDA, BxOpcodeInfo_FloatingPointDB,
        BxOpcodeInfo_FloatingPointDC, BxOpcodeInfo_FloatingPointDD,
        BxOpcodeInfo_FloatingPointDE, BxOpcodeInfo_FloatingPointDF,
    };
    
    // x87 FPU escape opcodes: D8-DF
    if !(0xd8..=0xdf).contains(&b1) {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }
    
    let rex = RexPrefix::from_byte(rex_prefix).unwrap_or_default();
    let rex_r = rex.rex_r();
    let rex_x = rex.rex_x();
    let rex_b = rex.rex_b();
    
    // Parse ModRM byte
    if iptr.is_empty() {
        return Err(BxDecodeError::ModRmParseFail.into());
    }
    
    let modrm_byte = iptr[0];
    iptr = &iptr[1..];
    
    let mod_field = u32::from(modrm_byte & 0xc0);
    let nnn = (((modrm_byte >> 3) & 0x7) as u32) | rex_r;
    let rm = ((modrm_byte & 0x7) as u32) | rex_b;
    
    // Decode ModRM for memory addressing if needed
    if mod_field != 0xc0 {
        iptr = decode_modrm64(iptr, i, mod_field, nnn, rm, rex_r, rex_x, rex_b)?;
    } else {
        i.assert_mod_c0();
    }
    
    // Store foo value for x87 instructions
    let foo = ((u16::from(modrm_byte)) | (u16::try_from(b1).unwrap_or(0) << 8)) & 0x7ff;
    i.set_foo(foo);
    
    // Select the appropriate x87 opcode table based on b1
    let x87_table = match b1 {
        0xd8 => &BxOpcodeInfo_FloatingPointD8[..],
        0xd9 => &BxOpcodeInfo_FloatingPointD9[..],
        0xda => &BxOpcodeInfo_FloatingPointDA[..],
        0xdb => &BxOpcodeInfo_FloatingPointDB[..],
        0xdc => &BxOpcodeInfo_FloatingPointDC[..],
        0xdd => &BxOpcodeInfo_FloatingPointDD[..],
        0xde => &BxOpcodeInfo_FloatingPointDE[..],
        0xdf => &BxOpcodeInfo_FloatingPointDF[..],
        _ => return Err(BxDecodeError::BxIllegalOpcode.into()),
    };
    
    // Determine opcode index
    let opcode_idx = if mod_field != 0xc0 {
        // /m form: use nnn directly (0-7)
        usize::try_from(nnn).unwrap_or(0)
    } else {
        // /r form: use (modrm & 0x3f) + 8
        usize::try_from(modrm_byte & 0x3f).unwrap_or(0) + 8
    };
    
    if opcode_idx >= x87_table.len() {
        return Err(BxDecodeError::BxIllegalOpcode.into());
    }
    
    let ia_opcode = x87_table[opcode_idx];
    
    // Assign sources
    fetchdecode32::assign_srcs(i, ia_opcode, nnn, rm)?;
    
    Ok((ia_opcode, iptr))
}

/// Decode 3DNow! instructions (64-bit)
/// 
/// Based on decoder64_3dnow from the C++ implementation.
#[cfg(feature = "3dnow")]
fn decoder64_3dnow<'a>(
    mut iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    use super::fetchdecode32;
    use super::fetchdecode_x87::Bx3DNowOpcode;
    
    let rex = RexPrefix::from_byte(rex_prefix).unwrap_or_default();
    let rex_r = rex.rex_r();
    let rex_x = rex.rex_x();
    let rex_b = rex.rex_b();
    
    // Parse ModRM
    let (modrm, updated_iptr) = parse_modrm64(iptr, i, rex_r, rex_x, rex_b)?;
    iptr = updated_iptr;
    
    // Fetch 3DNow! opcode suffix byte
    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }
    
    let ib_val = iptr[0];
    iptr = &iptr[1..];
    i.modrm_form.operand_data.set_ib([ib_val, 0, 0, 0]);
    
    let ia_opcode = Bx3DNowOpcode[usize::from(ib_val)];
    
    // Assign sources
    fetchdecode32::assign_srcs(i, ia_opcode, modrm.nnn, modrm.rm)?;
    
    Ok((ia_opcode, iptr))
}

#[cfg(not(feature = "3dnow"))]
fn decoder64_3dnow<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    Err(BxDecodeError::BxIllegalOpcode.into())
}

/// Decode NOP instruction (64-bit)
/// 
/// Based on decoder64_nop from the C++ implementation.
fn decoder64_nop<'a>(
    iptr: &'a [u8],
    i: &mut BxInstructionGenerated,
    b1: u32,
    sse_prefix: Option<SsePrefix>,
    rex_prefix: u8,
    opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    assert_eq!(b1, 0x90, "decoder64_nop: invalid b1 value");
    
    i.assert_mod_c0();
    
    let rex_b = (rex_prefix & 0x1) != 0;
    if rex_b {
        // REX.B set: decode as regular instruction (XCHG)
        decoder64(iptr, i, b1, sse_prefix, rex_prefix, opcode_table)
    } else {
        // Check for PAUSE instruction (F3 prefix)
        if sse_prefix == Some(SsePrefix::PrefixF3) {
            Ok((Opcode::Pause, iptr))
        } else {
            Ok((Opcode::Nop, iptr))
        }
    }
}

/// Decode VEX-prefixed AVX instructions (64-bit)
/// 
/// Based on decoder_vex64 from the C++ implementation.
#[cfg(feature = "avx")]
fn decoder_vex64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    // TODO: Implement decoder_vex64
    Err(BxDecodeError::BxIllegalOpcode.into())
}

#[cfg(not(feature = "avx"))]
fn decoder_vex64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    Err(BxDecodeError::BxIllegalOpcode.into())
}

/// Decode EVEX-prefixed AVX-512 instructions (64-bit)
/// 
/// Based on decoder_evex64 from the C++ implementation.
#[cfg(feature = "avx")]
fn decoder_evex64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    // TODO: Implement decoder_evex64
    Err(BxDecodeError::BxIllegalOpcode.into())
}

#[cfg(not(feature = "avx"))]
fn decoder_evex64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    Err(BxDecodeError::BxIllegalOpcode.into())
}

/// Decode XOP-prefixed instructions (64-bit)
/// 
/// Based on decoder_xop64 from the C++ implementation.
#[cfg(feature = "avx")]
fn decoder_xop64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    // TODO: Implement decoder_xop64
    Err(BxDecodeError::BxIllegalOpcode.into())
}

#[cfg(not(feature = "avx"))]
fn decoder_xop64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    Err(BxDecodeError::BxIllegalOpcode.into())
}

/// Decode undefined instruction (64-bit)
/// 
/// Based on decoder_ud64 from the C++ implementation.
fn decoder_ud64<'a>(
    _iptr: &'a [u8],
    _i: &mut BxInstructionGenerated,
    _b1: u32,
    _sse_prefix: Option<SsePrefix>,
    _rex_prefix: u8,
    _opcode_table: Option<&'static [u64]>,
) -> DecodeResult<(Opcode, &'a [u8])> {
    Err(BxDecodeError::BxIllegalOpcode.into())
}

/// Decode descriptor for 64-bit instructions
/// 
/// Maps opcode bytes to decoder functions and opcode tables.
struct BxOpcodeDecodeDescriptor64 {
    decode_method: for<'a> fn(&'a [u8], &mut BxInstructionGenerated, u32, Option<SsePrefix>, u8, Option<&'static [u64]>) -> DecodeResult<(Opcode, &'a [u8])>,
    opcode_table: &'static Option<&'static [u64]>,
}

/// 64-bit instruction decode descriptor array
/// 
/// Maps opcode bytes (0x00-0xFF for single-byte, 0x100-0x1FF for 0F escape) to decoder functions.
/// This array has 512 entries: 256 for single-byte opcodes and 256 for 0F escape opcodes.
/// 
/// Based on decode64_descriptor from cpp_orig/bochs/cpu/decoder/fetchdecode64.cc
pub(super) const DECODE64_DESCRIPTOR: [BxOpcodeDecodeDescriptor64; 512] = [
    // Single-byte opcodes (0x00-0xFF) - entries 0-255
    // 0F escape opcodes (0x100-0x1FF) - entries 256-511
    
    // Note: In Rust, we can't use Some(&TABLE) directly in const context.
    // We use const blocks to create the Option values.
    
    // 0x00-0x0F
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable00),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable01),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable02),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable03),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64,
        opcode_table: &Some(&BxOpcodeTable04),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64,
        opcode_table: &Some(&BxOpcodeTable05),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder_ud64,
        opcode_table: &Some(&BxOpcodeTable00), // TODO: Replace with actual table or None
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder_ud64,
        opcode_table: &None,
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable08),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable09),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable0A),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable0B),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64,
        opcode_table: &Some(&BxOpcodeTable0C),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64,
        opcode_table: &Some(&BxOpcodeTable0D),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder_ud64,
        opcode_table: &None,
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder_ud64,
        opcode_table: &None, // 0F escape
    },
    
    // 0x10-0x1F
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable10),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable11),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable12),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable13),
    },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable14) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable15) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable18) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable19) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable1A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable1B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable1C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable1D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    
    // 0x20-0x2F
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable20) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable21) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable22) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable23) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable24) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable25) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // ES:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable28) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable29) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable2A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable2B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable2C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable2D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // CS:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    
    // 0x30-0x3F
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable30) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable31) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable32) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable33) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable34) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable35) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // SS:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable38) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable39) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable3A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable3B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable3C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable3D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // DS:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    
    // 0x40-0x4F (REX prefixes)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REX prefix
    
    // 0x50-0x5F
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable50x57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable58x5F) },
    
    // 0x60-0x6F
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_evex64, opcode_table: &None, }, // EVEX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable63_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // FS:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // GS:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // OSIZE:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // ASIZE:
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable68) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable69) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable6A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable6B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable6C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable6D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable6E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable6F) },
    
    // 0x70-0x7F (conditional jumps - 64-bit specific)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable70_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable71_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable72_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable73_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable74_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable75_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable76_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable77_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable78_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable79_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable7A_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable7B_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable7C_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable7D_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable7E_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable7F_64) },
    
    // 0x80-0x8F
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable80) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable81) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable83) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable84) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable85) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable86) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable87) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable88) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable89) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable8A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable8B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable8C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable8D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable8E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_xop64, opcode_table: &Some(&BxOpcodeTable8F) }, // XOP prefix
    
    // 0x90-0x9F
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_nop, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable90x97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable98) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable99) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable9B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable9C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable9D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable9E_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable9F_64) },
    
    // 0xA0-0xAF
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA0_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA1_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA2_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA3_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableA9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableAA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableAB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableAC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableAD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableAE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableAF) },
    
    // 0xB0-0xBF
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB0xB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableB8xBF) },
    
    // 0xC0-0xCF
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableC0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableC1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableC2_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableC3_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_vex64, opcode_table: &None, }, // VEX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_vex64, opcode_table: &None, }, // VEX prefix
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableC6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableC7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableC8_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableC9_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableCA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableCB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableCC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableCD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableCF_64) },
    
    // 0xD0-0xDF
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableD0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableD1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableD2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableD3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableD7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_fp_escape, opcode_table: &None, },
    
    // 0xE0-0xEF
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE0_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE1_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE2_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE3_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE8_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableE9_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableEB_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableEC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableED) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableEE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTableEF) },
    
    // 0xF0-0xFF
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // LOCK
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableF1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REPNE/REPNZ
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, }, // REP, REPE/REPZ
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableF4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableF5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableF6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableF7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableF8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableF9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableFA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableFB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableFC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTableFD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableFE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableFF) },
    
    // 0F 00-0F 0F (entries 256-271)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F00) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F01) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F02) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F03) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F05_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F06) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F07_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F08) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F09) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F0B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F0D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F0E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_3dnow, opcode_table: &None, },
    
    // 0F 10-0F 1F (entries 272-287)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F10) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F11) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F12) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F13) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F14) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F15) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F16) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F17) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F18) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) }, // 0F 1E (CET support)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTableMultiByteNOP) },
    
    // 0F 20-0F 2F (entries 288-303)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_creg64, opcode_table: &Some(&BxOpcodeTable0F20_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_creg64, opcode_table: &Some(&BxOpcodeTable0F21_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_creg64, opcode_table: &Some(&BxOpcodeTable0F22_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_creg64, opcode_table: &Some(&BxOpcodeTable0F23_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F28) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F29) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F2A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F2B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F2C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F2D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F2E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F2F) },
    
    // 0F 30-0F 3F (entries 304-319)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F30) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F31) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F32) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F33) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F34) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0F35) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F37) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &None, }, // 0F 38 - 3-byte escape
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &None, }, // 0F 3A - 3-byte escape
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    
    // 0F 40-0F 4F (entries 320-335)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F40) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F41) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F42) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F43) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F44) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F45) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F46) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F47) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F48) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F49) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F4A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F4B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F4C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F4D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F4E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F4F) },
    
    // 0F 50-0F 5F (entries 336-351)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F50) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F51) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F52) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F53) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F54) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F55) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F56) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F57) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F58) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F59) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F5A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F5B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F5C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F5D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F5E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F5F) },
    
    // 0F 60-0F 6F (entries 352-367)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F60) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F61) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F62) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F63) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F65) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F66) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F67) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F68) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F69) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F6A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F6B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F6C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F6D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F6E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F6F) },
    
    // 0F 70-0F 7F (entries 368-383)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F70) },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable0F71),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable0F72),
    },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F73) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F74) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F75) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F76) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F77) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F78) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F79) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F7C) },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable0F7D),
    },
    BxOpcodeDecodeDescriptor64 {
        decode_method: decoder64_modrm,
        opcode_table: &Some(&BxOpcodeTable0F7E),
    },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F7F) },
    
    // 0F 80-0F 8F (entries 384-399) - conditional jumps (64-bit specific)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F80_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F81_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F82_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F83_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F84_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F85_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F86_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F87_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F88_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F89_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F8A_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F8B_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F8C_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F8D_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F8E_64) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0F8F_64) },
    
    // 0F 90-0F 9F (entries 400-415)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F90) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F91) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F92) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F93) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F94) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F95) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F96) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F97) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F98) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F99) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F9A) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F9B) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F9C) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F9D) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F9E) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0F9F) },
    
    // 0F A0-0F AF (entries 416-431)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FA0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FA1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0FA2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FA3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FA4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FA5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_ud64, opcode_table: &None, },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FA8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FA9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0FAA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FAB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FAC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FAD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FAE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FAF) },
    
    // 0F B0-0F BF (entries 432-447)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FB9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FBA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FBB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FBC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FBD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FBE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FBF) },
    
    // 0F C0-0F CF (entries 448-463)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FC7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64, opcode_table: &Some(&BxOpcodeTable0FC8x0FCF) },
    
    // 0F D0-0F DF (entries 464-479)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FD9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FDA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FDB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FDC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FDD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FDE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FDF) },
    
    // 0F E0-0F EF (entries 480-495)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FE9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FEA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FEB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FEC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FED) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FEE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FEF) },
    
    // 0F F0-0F FF (entries 496-511)
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF0) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF1) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF2) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF3) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF4) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF5) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF6) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF7) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF8) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FF9) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FFA) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FFB) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FFC) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FFD) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder64_modrm, opcode_table: &Some(&BxOpcodeTable0FFE) },
    BxOpcodeDecodeDescriptor64 { decode_method: decoder_simple64, opcode_table: &Some(&BxOpcodeTable0FFF) },
];

/// Main 64-bit instruction decoder
/// 
/// Decodes x86-64 instructions with REX prefix support, extended registers,
/// and RIP-relative addressing.
/// 
/// Based on fetchDecode64 from cpp_orig/bochs/cpu/decoder/fetchdecode64.cc
/// 
/// # Arguments
/// 
/// * `iptr` - Instruction pointer (byte slice)
/// 
/// # Returns
/// 
/// Decoded instruction or error
pub fn fetch_decode64(
    mut iptr: &[u8],
) -> DecodeResult<BxInstructionGenerated> {
    let remaining_in_page = iptr.len().min(15);
    iptr = &iptr[0..remaining_in_page];
    
    let mut instruction = BxInstructionGenerated::default();
    
    let mut b1: u32;
    let mut ia_opcode = Opcode::IaError;
    let mut seg_override: Option<BxSegregs> = None;
    let mut lock = false;
    let mut sse_prefix: Option<SsePrefix> = None;
    let mut rex_prefix: u8 = 0;
    
    // Initialize for 64-bit mode:
    // - os32 = 1 (operand size 32-bit override defaults to 1)
    // - as32 = 1 (address size 32-bit override defaults to 1)
    // - os64 = 0 (operand size 64-bit override defaults to 0)
    // - as64 = 1 (address size 64-bit override defaults to 1)
    instruction.init(1, 1, 0, 1);
    
    let mut meta_info_1 = MetaInfoFlags::default();
    meta_info_1.set_os32_b(true);
    meta_info_1.set(MetaInfoFlags::As32, true);
    meta_info_1.set(MetaInfoFlags::As64, true);
    
    if iptr.is_empty() {
        return Err(BxDecodeError::NoMoreLen.into());
    }
    
    // Prefix parsing loop
    loop {
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        
        b1 = u32::from(iptr[0]);
        iptr = &iptr[1..];
        
        match b1 {
            // REX prefix (0x40-0x4F)
            0x40..=0x4F => {
                rex_prefix = b1 as u8;
                // Continue to next byte
            }
            // 2-byte escape (0x0F)
            0x0f => {
                if iptr.is_empty() {
                    return Err(BxDecodeError::NoMoreLen.into());
                }
                b1 = 0x100 | u32::from(iptr[0]);
                iptr = &iptr[1..];
                break;
            }
            // REPNE/REPNZ (0xF2)
            0xf2 => {
                rex_prefix = 0; // REX prefix must come before REP prefixes
                sse_prefix = Some(SsePrefix::PrefixF2);
                meta_info_1.set_lock_rep_used(2);
            }
            // REP/REPE/REPZ (0xF3)
            0xf3 => {
                rex_prefix = 0;
                sse_prefix = Some(SsePrefix::PrefixF3);
                meta_info_1.set_lock_rep_used(3);
            }
            // Segment overrides (CS, DS, ES, SS are ignored in 64-bit mode)
            0x26 | 0x2e | 0x36 | 0x3e => {
                rex_prefix = 0;
                // Ignored in 64-bit mode, but continue parsing
            }
            // FS: segment override (0x64)
            0x64 => {
                rex_prefix = 0;
                seg_override = Some(BxSegregs::Fs);
            }
            // GS: segment override (0x65)
            0x65 => {
                rex_prefix = 0;
                seg_override = Some(BxSegregs::Gs);
            }
            // OpSize prefix (0x66)
            0x66 => {
                rex_prefix = 0;
                if sse_prefix.is_none() {
                    sse_prefix = Some(SsePrefix::Prefix66);
                }
                meta_info_1.set_os32_b(false);
            }
            // AddrSize prefix (0x67)
            0x67 => {
                rex_prefix = 0;
                instruction.clear_as64();
            }
            // LOCK prefix (0xF0)
            0xf0 => {
                rex_prefix = 0;
                lock = true;
            }
            // Not a prefix - this is the opcode
            _ => {
                break;
            }
        }
    }
    
    // Apply REX prefix effects
    if let Some(rex) = RexPrefix::from_byte(rex_prefix) {
        instruction.assert_extend8bit();
        if rex.w {
            instruction.assert_os64();
            instruction.assert_os32();
            meta_info_1.set(MetaInfoFlags::Os64, true);
            meta_info_1.set(MetaInfoFlags::Os32, true);
        }
    }
    
    // Get decode descriptor
    // Use DECODE64_DESCRIPTOR for 64-bit mode
    // For 0F escape opcodes, add 256 to the index
    let descriptor_idx = if b1 >= 0x100 {
        // 0F escape opcode: index = 256 + (b1 & 0xFF)
        (256 + (b1 & 0xFF)) as usize
    } else {
        b1 as usize
    };
    
    let decode_descriptor = &DECODE64_DESCRIPTOR[descriptor_idx.min(511)];
    let decode_method = decode_descriptor.decode_method;
    let mut opcode_table = *decode_descriptor.opcode_table;
    
    // Handle 3-byte opcodes (0F 38, 0F 3A)
    if b1 == 0x138 || b1 == 0x13a {
        if iptr.is_empty() {
            return Err(BxDecodeError::NoMoreLen.into());
        }
        let opcode = iptr[0];
        iptr = &iptr[1..];
        
        if b1 == 0x138 {
            opcode_table = Some(BxOpcodeTable0F38[opcode as usize]);
            b1 = 0x200 | u32::from(opcode);
        } else if b1 == 0x13a {
            opcode_table = Some(BxOpcodeTable0F3A[opcode as usize]);
            b1 = 0x300 | u32::from(opcode);
        }
    }
    
    // Set default segment (DS:)
    instruction.set_seg(BxSegregs::Ds);
    instruction.set_cet_seg_override(BxSegregs::Null);
    
    instruction.modrm_form.operand_data.set_id(0);
    
    // Call decoder function from descriptor
    let decode_method = decode_descriptor.decode_method;
    let descriptor_opcode_table = *decode_descriptor.opcode_table;
    
    // Use opcode table from descriptor if available, otherwise use the one from 3-byte escape handling
    let final_opcode_table = descriptor_opcode_table.or(opcode_table);
    
    (ia_opcode, iptr) = decode_method(iptr, &mut instruction, b1, sse_prefix, rex_prefix, final_opcode_table)?;
    
    instruction.meta_info.metainfo1 = meta_info_1;
    instruction.meta_info.ia_opcode = ia_opcode;
    instruction.meta_info.ilen = u8::try_from(remaining_in_page)
        .unwrap_or(0)
        .saturating_sub(u8::try_from(iptr.len()).unwrap_or(0));
    
    // Apply segment override (only FS: and GS: are valid in 64-bit mode)
    if let Some(seg) = seg_override {
        if matches!(seg, BxSegregs::Fs | BxSegregs::Gs) {
            instruction.set_seg(seg);
        }
    }
    
    // Handle LOCK prefix
    if lock {
        instruction.set_lock();
        // TODO: Validate lock prefix (check op_flags from BxOpcodesTable)
        // For now, just set the lock flag
    }
    
    Ok(instruction)
}

