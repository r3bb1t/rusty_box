//! SSE/SSE2 packed integer instruction handlers (128-bit XMM)
//!
//! Based on Bochs cpu/sse_int.cc and cpu/sse_move.cc
//!
//! Implements SSE2 128-bit packed integer operations including:
//! - Packed add/sub (PADDB/W/D/Q, PSUBB/W/D/Q)
//! - Saturating add/sub (PADDS/PADDUS/PSUBS/PSUBUS B/W)
//! - Multiply (PMULLW, PMULHW, PMULHUW, PMULUDQ, PMADDWD)
//! - Compare (PCMPEQB/W/D, PCMPGTB/W/D)
//! - Logical (PAND, PANDN, POR, PXOR)
//! - Shift by XMM/immediate (PSRL/PSRA/PSLL W/D/Q, PSLLDQ, PSRLDQ)
//! - Pack/Unpack (PUNPCKL/H B/W/D/Q, PACKSSWB/PACKSSDW/PACKUSWB)
//! - Shuffle (PSHUFD, PSHUFHW, PSHUFLW)
//! - Insert/Extract (PINSRW, PEXTRW)
//! - Min/Max/Average (PMINUB, PMAXUB, PMINSW, PMAXSW, PAVGB, PAVGW)
//! - Misc (PMOVMSKB, PSADBW, MASKMOVDQU)

use super::{
    decoder::{BxSegregs, Instruction},
    simd_int,
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// Saturation helpers (matching Bochs sse_int.cc / mmx.cc inline functions)
// ============================================================================

/// Saturate a signed 16-bit value to signed 8-bit range [-128, 127]
#[inline]
pub(super) fn saturate_word_s_to_byte_s(val: i16) -> i8 {
    if val > 127 {
        127
    } else if val < -128 {
        -128
    } else {
        val as i8
    }
}

/// Saturate a signed 16-bit value to unsigned 8-bit range [0, 255]
#[inline]
pub(super) fn saturate_word_s_to_byte_u(val: i16) -> u8 {
    if val > 255 {
        255
    } else if val < 0 {
        0
    } else {
        val as u8
    }
}

/// Saturate a signed 32-bit value to signed 16-bit range [-32768, 32767]
#[inline]
pub(super) fn saturate_dword_s_to_word_s(val: i32) -> i16 {
    if val > 32767 {
        32767
    } else if val < -32768 {
        -32768
    } else {
        val as i16
    }
}

/// Bochs sse.cc `xmm_extrq`: the field `len` bits long at bit `shift` of
/// `src`, moved to bit 0. Both are taken modulo 64, and a length of 0 keeps
/// every bit from `shift` up.
fn xmm_extrq(src: u64, shift: u8, len: u8) -> u64 {
    let len = len & 0x3f;
    let shift = shift & 0x3f;
    let src = src >> shift;
    if len != 0 {
        src & ((1u64 << len) - 1)
    } else {
        src
    }
}

/// Bochs sse.cc `xmm_insertq`: `dest` with the field `len` bits long at bit
/// `shift` replaced by the low `len` bits of `src`. Both are taken modulo 64,
/// and a length of 0 means all 64.
fn xmm_insertq(dest: u64, src: u64, shift: u8, len: u8) -> u64 {
    let len = len & 0x3f;
    let shift = shift & 0x3f;
    let mask = if len == 0 { u64::MAX } else { (1u64 << len) - 1 };
    (dest & !(mask << shift)) | ((src & mask) << shift)
}

/// Saturate a signed 32-bit value to unsigned 16-bit range [0, 65535] —
/// Bochs xmm.h `SaturateDwordSToWordU`.
#[inline]
pub(super) fn saturate_dword_s_to_word_u(val: i32) -> u16 {
    if val < 0 {
        0
    } else if val > 65535 {
        65535
    } else {
        val as u16
    }
}

// ============================================================================
// SSE4.1 blend lane helpers (Bochs simd_int.h xmm_blendps/xmm_blendpd/
// xmm_blendvps/xmm_blendvpd). Shared by the legacy handlers below and the
// per-128-bit-lane VEX handlers in avx_pfp.rs.
// ============================================================================

/// Bochs simd_int.h xmm_blendps: copy op2 dword lanes selected by mask[3:0]
#[inline]
pub(super) fn blendps_lane(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister, mask: u8) {
    for n in 0..4usize {
        if mask & (1 << n) != 0 {
            op1.set_xmm32u(n, op2.xmm32u(n));
        }
    }
}

/// Bochs simd_int.h xmm_blendpd: copy op2 qword lanes selected by mask[1:0]
#[inline]
pub(super) fn blendpd_lane(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister, mask: u8) {
    for n in 0..2usize {
        if mask & (1 << n) != 0 {
            op1.set_xmm64u(n, op2.xmm64u(n));
        }
    }
}

/// Bochs simd_int.h xmm_blendvps: copy op2 dword lanes whose mask-register
/// lane has the sign bit set
#[inline]
pub(super) fn blendvps_lane(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: &BxPackedXmmRegister,
) {
    for n in 0..4usize {
        if mask.xmm32s(n) < 0 {
            op1.set_xmm32u(n, op2.xmm32u(n));
        }
    }
}

/// Bochs simd_int.h xmm_blendvpd: copy op2 qword lanes whose mask-register
/// lane has the sign bit (bit 63 = sign of the high dword) set
#[inline]
pub(super) fn blendvpd_lane(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: &BxPackedXmmRegister,
) {
    if mask.xmm32s(1) < 0 {
        op1.set_xmm64u(0, op2.xmm64u(0));
    }
    if mask.xmm32s(3) < 0 {
        op1.set_xmm64u(1, op2.xmm64u(1));
    }
}

/// Bochs simd_int.h xmm_pblendvb: copy op2 byte lanes whose mask-register
/// byte has the sign bit set
#[inline]
pub(super) fn pblendvb_lane(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: &BxPackedXmmRegister,
) {
    for n in 0..16usize {
        if mask.xmm_sbyte(n) < 0 {
            op1.set_xmmubyte(n, op2.xmmubyte(n));
        }
    }
}

/// Bochs simd_int.h xmm_pabsb: per-byte absolute value (|-128| stays 0x80)
#[inline]
pub(super) fn pabsb_lane(op: &mut BxPackedXmmRegister) {
    for n in 0..16usize {
        op.set_xmm_sbyte(n, op.xmm_sbyte(n).wrapping_abs());
    }
}

/// Bochs simd_int.h xmm_pabsw: per-word absolute value (|-32768| stays 0x8000)
#[inline]
pub(super) fn pabsw_lane(op: &mut BxPackedXmmRegister) {
    for n in 0..8usize {
        op.set_xmm16s(n, op.xmm16s(n).wrapping_abs());
    }
}

/// Bochs simd_int.h xmm_pabsd: per-dword absolute value
#[inline]
pub(super) fn pabsd_lane(op: &mut BxPackedXmmRegister) {
    for n in 0..4usize {
        op.set_xmm32s(n, op.xmm32s(n).wrapping_abs());
    }
}

/// Bochs simd_int.h xmm_mpsadbw (via sad_quadruple): eight overlapping
/// 4-byte sums of absolute differences. `control` bits [1:0] select the
/// op2 quadruple, bit [2] the op1 window base.
#[inline]
pub(super) fn mpsadbw_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    control: u8,
) -> BxPackedXmmRegister {
    let src_offset = ((control & 0x3) as usize) * 4;
    let dst_offset = (((control >> 2) & 0x1) as usize) * 4;
    let mut r = BxPackedXmmRegister::default();
    for j in 0..8usize {
        let mut sad = 0u16;
        for n in 0..4usize {
            let a = op1.xmmubyte(dst_offset + j + n) as i16;
            let b = op2.xmmubyte(src_offset + n) as i16;
            sad = sad.wrapping_add((a - b).unsigned_abs());
        }
        r.set_xmm16u(j, sad);
    }
    r
}

/// Bochs sse.cc PHMINPOSUW_VdqWdqR core: find the minimum unsigned word;
/// result word 0 = minimum value, word 1 = its index, rest zero.
#[inline]
pub(super) fn phminposuw_core(op: &BxPackedXmmRegister) -> BxPackedXmmRegister {
    let mut min_index = 0usize;
    for j in 1..8usize {
        if op.xmm16u(j) < op.xmm16u(min_index) {
            min_index = j;
        }
    }
    let mut r = BxPackedXmmRegister::default();
    r.set_xmm16u(0, op.xmm16u(min_index));
    r.set_xmm16u(1, min_index as u16);
    r
}

/// Bochs sse.cc INSERTPS core (insert + simd_int.h xmm_zero_blendps):
/// write `op2` into the dword selected by imm[5:4], then zero every dword
/// whose imm[3:0] bit is set.
#[inline]
pub(super) fn insertps_core(op1: &mut BxPackedXmmRegister, op2: u32, control: u8) {
    op1.set_xmm32u(((control >> 4) & 3) as usize, op2);
    for n in 0..4usize {
        if control & (1 << n) != 0 {
            op1.set_xmm32u(n, 0);
        }
    }
}

impl<T: crate::cpu::instrumentation::Instrumentation> crate::cpu::exec_ctx::ExecCtx<'_, T> {
    // ========================================================================
    // SSE helper: read op2 (register or memory)
    // ========================================================================

    /// The 128-bit second operand of a legacy SSE instruction — Bochs load.cc
    /// `LOAD_Wdq`, the memory-form loader of every legacy SSE opcode with a
    /// 128-bit memory operand except the four string compares.
    ///
    /// The memory form must be 16-byte aligned, and raises #GP(0) otherwise,
    /// unless MXCSR.MM allows misaligned SSE. A VEX encoding the decoder routes
    /// to the same handler — the shared legacy tables keep some VEX forms, AES
    /// among them, on their legacy opcode — is Bochs's `LOAD_Vector` instead,
    /// which never checks alignment.
    #[inline]
    pub(super) fn sse_read_op2_xmm(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            return Ok(self.read_xmm_reg(instr.src1()));
        }
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        if instr.is_vex() || self.mxcsr.misaligned_sse() {
            self.v_read_xmmword(seg, eaddr)
        } else {
            self.v_read_xmmword_aligned(seg, eaddr)
        }
    }

    /// Bochs cpu_templates.h `HANDLE_SSE_2OP`: a legacy SSE instruction whose
    /// result is `func` applied to its destination and second operand.
    fn sse_2op(&mut self, instr: &Instruction, func: simd_int::Xmm2Op) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        func(&mut op1, &op2);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// The same operand with no alignment rule — Bochs load.cc `LOADU_Wdq`,
    /// the memory-form loader of PCMPESTRI, PCMPESTRM, PCMPISTRI and
    /// PCMPISTRM in their legacy and VEX encodings alike.
    #[inline]
    pub(super) fn sse_read_op2_xmm_unaligned(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            return Ok(self.read_xmm_reg(instr.src1()));
        }
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        self.v_read_xmmword(seg, eaddr)
    }

    // ========================================================================
    // Packed Add (PADDB/W/D/Q) — SSE2 128-bit
    // Bochs sse_int.cc
    // ========================================================================

    /// PADDB VdqWdq — packed add bytes (16 x u8)
    pub(super) fn paddb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(i, op1.xmmubyte(i).wrapping_add(op2.xmmubyte(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PADDW VdqWdq — packed add words (8 x u16)
    pub(super) fn paddw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(i, op1.xmm16u(i).wrapping_add(op2.xmm16u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PADDD VdqWdq — packed add dwords (4 x u32)
    pub(super) fn paddd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32u(i, op1.xmm32u(i).wrapping_add(op2.xmm32u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PADDQ VdqWdq — packed add qwords (2 x u64)
    pub(super) fn paddq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0).wrapping_add(op2.xmm64u(0)));
        result.set_xmm64u(1, op1.xmm64u(1).wrapping_add(op2.xmm64u(1)));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Packed Sub (PSUBB/W/D/Q) — SSE2 128-bit
    // ========================================================================

    /// PSUBB VdqWdq — packed sub bytes (16 x u8)
    pub(super) fn psubb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(i, op1.xmmubyte(i).wrapping_sub(op2.xmmubyte(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSUBW VdqWdq — packed sub words (8 x u16)
    pub(super) fn psubw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(i, op1.xmm16u(i).wrapping_sub(op2.xmm16u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSUBD VdqWdq — packed sub dwords (4 x u32)
    pub(super) fn psubd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32u(i, op1.xmm32u(i).wrapping_sub(op2.xmm32u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSUBQ VdqWdq — packed sub qwords (2 x u64)
    pub(super) fn psubq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0).wrapping_sub(op2.xmm64u(0)));
        result.set_xmm64u(1, op1.xmm64u(1).wrapping_sub(op2.xmm64u(1)));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Saturating Add — signed and unsigned (PADDSB/W, PADDUSB/W)
    // ========================================================================

    /// PADDSB VdqWdq — packed add signed bytes with saturation
    pub(super) fn paddsb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmm_sbyte(
                i,
                saturate_word_s_to_byte_s(op1.xmm_sbyte(i) as i16 + op2.xmm_sbyte(i) as i16),
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PADDSW VdqWdq — packed add signed words with saturation
    pub(super) fn paddsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16s(
                i,
                saturate_dword_s_to_word_s(op1.xmm16s(i) as i32 + op2.xmm16s(i) as i32),
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PADDUSB VdqWdq — packed add unsigned bytes with saturation
    pub(super) fn paddusb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(i, op1.xmmubyte(i).saturating_add(op2.xmmubyte(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PADDUSW VdqWdq — packed add unsigned words with saturation
    pub(super) fn paddusw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(i, op1.xmm16u(i).saturating_add(op2.xmm16u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Saturating Sub — signed and unsigned (PSUBSB/W, PSUBUSB/W)
    // ========================================================================

    /// PSUBSB VdqWdq — packed sub signed bytes with saturation
    pub(super) fn psubsb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmm_sbyte(
                i,
                saturate_word_s_to_byte_s(op1.xmm_sbyte(i) as i16 - op2.xmm_sbyte(i) as i16),
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSUBSW VdqWdq — packed sub signed words with saturation
    pub(super) fn psubsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16s(
                i,
                saturate_dword_s_to_word_s(op1.xmm16s(i) as i32 - op2.xmm16s(i) as i32),
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSUBUSB VdqWdq — packed sub unsigned bytes with saturation
    pub(super) fn psubusb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(i, op1.xmmubyte(i).saturating_sub(op2.xmmubyte(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSUBUSW VdqWdq — packed sub unsigned words with saturation
    pub(super) fn psubusw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(i, op1.xmm16u(i).saturating_sub(op2.xmm16u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Multiply (PMULLW, PMULHW, PMULHUW, PMULUDQ, PMADDWD)
    // ========================================================================

    /// PMULLW VdqWdq — packed multiply low words (8 x i16, keep low 16 bits)
    pub(super) fn pmullw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(
                i,
                (op1.xmm16u(i) as u32).wrapping_mul(op2.xmm16u(i) as u32) as u16,
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMULHW VdqWdq — packed multiply high signed words (8 x i16, keep high 16 bits)
    pub(super) fn pmulhw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(
                i,
                ((op1.xmm16s(i) as i32 * op2.xmm16s(i) as i32) >> 16) as u16,
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMULHUW VdqWdq — packed multiply high unsigned words (8 x u16, keep high 16 bits)
    pub(super) fn pmulhuw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(
                i,
                ((op1.xmm16u(i) as u32 * op2.xmm16u(i) as u32) >> 16) as u16,
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMULHRSW VdqWdq — packed multiply high with rounding and scale (SSSE3)
    /// Bochs simd_int.h: ((a * b >> 14) + 1) >> 1
    pub(super) fn pmulhrsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            let t = ((op1.xmm16s(i) as i32 * op2.xmm16s(i) as i32) >> 14) + 1;
            result.set_xmm16u(i, (t >> 1) as u16);
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMULUDQ VdqWdq — packed multiply unsigned dwords to qwords
    /// Multiplies dwords [0] and [2] of each operand, producing two 64-bit results.
    pub(super) fn pmuludq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, (op1.xmm32u(0) as u64) * (op2.xmm32u(0) as u64));
        result.set_xmm64u(1, (op1.xmm32u(2) as u64) * (op2.xmm32u(2) as u64));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMADDWD VdqWdq — multiply and add packed words to dwords
    /// (Bochs `HANDLE_SSE_2OP<xmm_pmaddwd>`).
    pub(super) fn pmaddwd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pmaddwd)
    }

    // ========================================================================
    // Compare (PCMPEQB/W/D, PCMPGTB/W/D)
    // ========================================================================

    /// PCMPEQB VdqWdq — packed compare equal bytes
    pub(super) fn pcmpeqb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(
                i,
                if op1.xmmubyte(i) == op2.xmmubyte(i) {
                    0xff
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PCMPEQW VdqWdq — packed compare equal words
    pub(super) fn pcmpeqw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(
                i,
                if op1.xmm16u(i) == op2.xmm16u(i) {
                    0xffff
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PCMPEQD VdqWdq — packed compare equal dwords
    pub(super) fn pcmpeqd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32u(
                i,
                if op1.xmm32u(i) == op2.xmm32u(i) {
                    0xffffffff
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PCMPGTB VdqWdq — packed compare greater than bytes (signed)
    pub(super) fn pcmpgtb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(
                i,
                if op1.xmm_sbyte(i) > op2.xmm_sbyte(i) {
                    0xff
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PCMPGTW VdqWdq — packed compare greater than words (signed)
    pub(super) fn pcmpgtw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16u(
                i,
                if op1.xmm16s(i) > op2.xmm16s(i) {
                    0xffff
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PCMPGTD VdqWdq — packed compare greater than dwords (signed)
    pub(super) fn pcmpgtd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32u(
                i,
                if op1.xmm32s(i) > op2.xmm32s(i) {
                    0xffffffff
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Logical (PAND, PANDN, POR, PXOR) — 128-bit
    // ========================================================================

    /// PAND VdqWdq — bitwise AND
    pub(super) fn pand_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) & op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) & op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PANDN VdqWdq — bitwise AND NOT (~op1 & op2)
    pub(super) fn pandn_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, !op1.xmm64u(0) & op2.xmm64u(0));
        result.set_xmm64u(1, !op1.xmm64u(1) & op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// POR VdqWdq — bitwise OR
    pub(super) fn por_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) | op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) | op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PXOR VdqWdq — bitwise XOR
    pub(super) fn pxor_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) ^ op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) ^ op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Shift by XMM register (PSRLW/D/Q, PSRAW/D, PSLLW/D/Q)
    // Shift count is in the low 64 bits of the source XMM.
    // ========================================================================

    /// PSRLW VdqWdq — shift right logical words by XMM count
    pub(super) fn psrlw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        if count > 15 {
            op1 = BxPackedXmmRegister::default();
        } else {
            let shift = count as u16;
            for i in 0..8 {
                op1.set_xmm16u(i, op1.xmm16u(i) >> shift);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSRLD VdqWdq — shift right logical dwords by XMM count
    pub(super) fn psrld_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        if count > 31 {
            op1 = BxPackedXmmRegister::default();
        } else {
            let shift = count as u32;
            for i in 0..4 {
                op1.set_xmm32u(i, op1.xmm32u(i) >> shift);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSRLQ VdqWdq — shift right logical qwords by XMM count
    pub(super) fn psrlq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        // Any count above 63 clears the destination (SDM), the bound the VEX
        // and EVEX forms use. Bochs simd_int.h xmm_psrlq clears only for
        // `> 64`, which leaves a count of 64 to a C++ shift by the operand's
        // full width — undefined behaviour; the port follows the SDM there,
        // registered as D9 in docs/bochs-parity-divergences.md.
        if count > 63 {
            op1 = BxPackedXmmRegister::default();
        } else {
            let shift = count as u32;
            op1.set_xmm64u(0, op1.xmm64u(0) >> shift);
            op1.set_xmm64u(1, op1.xmm64u(1) >> shift);
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSRAW VdqWdq — shift right arithmetic words by XMM count
    pub(super) fn psraw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        if count == 0 {
            // no change
        } else if count > 15 {
            for i in 0..8 {
                op1.set_xmm16u(i, if op1.xmm16s(i) < 0 { 0xffff } else { 0 });
            }
        } else {
            for i in 0..8 {
                op1.set_xmm16u(i, (op1.xmm16s(i) >> count as u16) as u16);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSRAD VdqWdq — shift right arithmetic dwords by XMM count
    pub(super) fn psrad_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        if count == 0 {
            // no change
        } else if count > 31 {
            for i in 0..4 {
                op1.set_xmm32u(i, if op1.xmm32s(i) < 0 { 0xffffffff } else { 0 });
            }
        } else {
            for i in 0..4 {
                op1.set_xmm32u(i, (op1.xmm32s(i) >> count as u32) as u32);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSLLW VdqWdq — shift left logical words by XMM count
    pub(super) fn psllw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        if count > 15 {
            op1 = BxPackedXmmRegister::default();
        } else {
            for i in 0..8 {
                op1.set_xmm16u(i, op1.xmm16u(i) << count as u16);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSLLD VdqWdq — shift left logical dwords by XMM count
    pub(super) fn pslld_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        if count > 31 {
            op1 = BxPackedXmmRegister::default();
        } else {
            for i in 0..4 {
                op1.set_xmm32u(i, op1.xmm32u(i) << count as u32);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PSLLQ VdqWdq — shift left logical qwords by XMM count
    pub(super) fn psllq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let count = op2.xmm64u(0);
        // Bochs simd_int.h xmm_psllq: any count above 63 clears the
        // destination.
        if count > 63 {
            op1 = BxPackedXmmRegister::default();
        } else {
            let shift = count as u32;
            op1.set_xmm64u(0, op1.xmm64u(0) << shift);
            op1.set_xmm64u(1, op1.xmm64u(1) << shift);
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    // ========================================================================
    // PSLLDQ / PSRLDQ — byte-shift entire 128-bit register by imm8
    // ========================================================================

    /// PSLLDQ UdqIb — shift left logical 128-bit by imm8 bytes (fills zeros from right)
    pub(super) fn pslldq_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.dst());
        let count = (instr.ib() as usize).min(16);

        let mut result = BxPackedXmmRegister::default();
        for i in count..16 {
            result.set_xmmubyte(i, op.xmmubyte(i - count));
        }
        // bytes 0..count remain zero (from default)
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSRLDQ UdqIb — shift right logical 128-bit by imm8 bytes (fills zeros from left)
    pub(super) fn psrldq_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.dst());
        let count = (instr.ib() as usize).min(16);

        let mut result = BxPackedXmmRegister::default();
        for i in count..16 {
            result.set_xmmubyte(i - count, op.xmmubyte(i));
        }
        // bytes (16-count)..16 remain zero (from default)
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Immediate shifts on dst XMM (PSRLW/D/Q, PSRAW/D, PSLLW/D/Q UdqIb)
    // ========================================================================

    /// PSRLW UdqIb — shift right logical words by imm8
    pub(super) fn psrlw_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        if shift > 15 {
            op = BxPackedXmmRegister::default();
        } else {
            for i in 0..8 {
                op.set_xmm16u(i, op.xmm16u(i) >> shift as u16);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSRLD UdqIb — shift right logical dwords by imm8
    pub(super) fn psrld_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        if shift > 31 {
            op = BxPackedXmmRegister::default();
        } else {
            for i in 0..4 {
                op.set_xmm32u(i, op.xmm32u(i) >> shift as u32);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSRLQ UdqIb — shift right logical qwords by imm8
    pub(super) fn psrlq_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        // Any count above 63 clears the destination (SDM), the bound the VEX
        // and EVEX forms use. Bochs simd_int.h xmm_psrlq clears only for
        // `> 64`, which leaves a count of 64 to a C++ shift by the operand's
        // full width — undefined behaviour; the port follows the SDM there,
        // registered as D9 in docs/bochs-parity-divergences.md.
        if shift > 63 {
            op = BxPackedXmmRegister::default();
        } else {
            let s = u32::from(shift);
            op.set_xmm64u(0, op.xmm64u(0) >> s);
            op.set_xmm64u(1, op.xmm64u(1) >> s);
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSRAW UdqIb — shift right arithmetic words by imm8
    pub(super) fn psraw_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        if shift == 0 {
            // no change
        } else if shift > 15 {
            for i in 0..8 {
                op.set_xmm16u(i, if op.xmm16s(i) < 0 { 0xffff } else { 0 });
            }
        } else {
            for i in 0..8 {
                op.set_xmm16u(i, (op.xmm16s(i) >> shift as i16) as u16);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSRAD UdqIb — shift right arithmetic dwords by imm8
    pub(super) fn psrad_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        if shift == 0 {
            // no change
        } else if shift > 31 {
            for i in 0..4 {
                op.set_xmm32u(i, if op.xmm32s(i) < 0 { 0xffffffff } else { 0 });
            }
        } else {
            for i in 0..4 {
                op.set_xmm32u(i, (op.xmm32s(i) >> shift as i32) as u32);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSLLW UdqIb — shift left logical words by imm8
    pub(super) fn psllw_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        if shift > 15 {
            op = BxPackedXmmRegister::default();
        } else {
            for i in 0..8 {
                op.set_xmm16u(i, op.xmm16u(i) << shift as u16);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSLLD UdqIb — shift left logical dwords by imm8
    pub(super) fn pslld_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        if shift > 31 {
            op = BxPackedXmmRegister::default();
        } else {
            for i in 0..4 {
                op.set_xmm32u(i, op.xmm32u(i) << shift as u32);
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PSLLQ UdqIb — shift left logical qwords by imm8
    pub(super) fn psllq_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.read_xmm_reg(instr.dst());
        let shift = instr.ib();

        // Bochs simd_int.h xmm_psllq: any count above 63 clears the
        // destination.
        if shift > 63 {
            op = BxPackedXmmRegister::default();
        } else {
            let s = u32::from(shift);
            op.set_xmm64u(0, op.xmm64u(0) << s);
            op.set_xmm64u(1, op.xmm64u(1) << s);
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // Unpack Low (PUNPCKLBW/WD/DQ/QDQ) — 128-bit SSE2
    // Uses LOW half of both operands, interleaves into full 128 bits.
    // ========================================================================

    /// PUNPCKLBW VdqWdq — unpack and interleave low bytes
    /// dst[0]=dst_orig[0], dst[1]=src[0], dst[2]=dst_orig[1], dst[3]=src[1], ...
    pub(super) fn punpcklbw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmmubyte(i * 2, op1.xmmubyte(i));
            result.set_xmmubyte(i * 2 + 1, op2.xmmubyte(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLWD VdqWdq — unpack and interleave low words
    pub(super) fn punpcklwd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm16u(i * 2, op1.xmm16u(i));
            result.set_xmm16u(i * 2 + 1, op2.xmm16u(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLDQ VdqWdq — unpack and interleave low dwords
    pub(super) fn punpckldq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32u(0, op1.xmm32u(0));
        result.set_xmm32u(1, op2.xmm32u(0));
        result.set_xmm32u(2, op1.xmm32u(1));
        result.set_xmm32u(3, op2.xmm32u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKLQDQ VdqWdq — unpack and interleave low qwords
    pub(super) fn punpcklqdq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0));
        result.set_xmm64u(1, op2.xmm64u(0));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Unpack High (PUNPCKHBW/WD/DQ/QDQ) — 128-bit SSE2
    // Uses HIGH half of both operands (bytes 8-15, words 4-7, etc.)
    // ========================================================================

    /// PUNPCKHBW VdqWdq — unpack and interleave high bytes
    pub(super) fn punpckhbw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmmubyte(i * 2, op1.xmmubyte(i + 8));
            result.set_xmmubyte(i * 2 + 1, op2.xmmubyte(i + 8));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHWD VdqWdq — unpack and interleave high words
    pub(super) fn punpckhwd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm16u(i * 2, op1.xmm16u(i + 4));
            result.set_xmm16u(i * 2 + 1, op2.xmm16u(i + 4));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHDQ VdqWdq — unpack and interleave high dwords
    pub(super) fn punpckhdq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32u(0, op1.xmm32u(2));
        result.set_xmm32u(1, op2.xmm32u(2));
        result.set_xmm32u(2, op1.xmm32u(3));
        result.set_xmm32u(3, op2.xmm32u(3));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PUNPCKHQDQ VdqWdq — unpack and interleave high qwords
    pub(super) fn punpckhqdq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(1));
        result.set_xmm64u(1, op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Pack (PACKSSWB, PACKSSDW, PACKUSWB) — 128-bit SSE2
    // ========================================================================

    /// PACKSSWB VdqWdq — pack signed words to signed bytes with saturation
    pub(super) fn packsswb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_packsswb)
    }

    /// PACKSSDW VdqWdq — pack signed dwords to signed words with saturation
    pub(super) fn packssdw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_packssdw)
    }

    /// PACKUSWB VdqWdq — pack signed words to unsigned bytes with saturation
    pub(super) fn packuswb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_packuswb)
    }

    /// PACKUSDW VdqWdq (66 0F 38 2B) — pack signed dwords to unsigned words
    /// with saturation (SSE4.1; Bochs `HANDLE_SSE_2OP<xmm_packusdw>`)
    pub(super) fn packusdw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_packusdw)
    }

    // ========================================================================
    // SSE4A (AMD) bit-field extract and insert — Bochs sse.cc. Each writes the
    // low quadword of its destination and leaves the rest of the register
    // alone (`BX_WRITE_XMM_REG_LO_QWORD`).
    // ========================================================================

    /// EXTRQ Udq, Ib, Ib2 (66 0F 78 /0 ib ib) — extract the field `Ib` bits
    /// long at bit `Ib2` of the register's low quadword into its low bits.
    pub(super) fn extrq_udq_ib_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let source = self.read_xmm_reg(instr.dst()).xmm64u(0);
        self.write_xmm_lo_qword(instr.dst(), xmm_extrq(source, instr.ib2(), instr.ib()));
        Ok(())
    }

    /// EXTRQ Vdq, Uq (66 0F 79 /r) — as above, with the length in bits 5:0
    /// and the position in bits 13:8 of the source register.
    pub(super) fn extrq_vdq_uq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let control = self.read_xmm_reg(instr.src1()).xmm16u(0);
        let source = self.read_xmm_reg(instr.dst()).xmm64u(0);
        let extracted = xmm_extrq(source, (control >> 8) as u8, control as u8);
        self.write_xmm_lo_qword(instr.dst(), extracted);
        Ok(())
    }

    /// INSERTQ Vdq, Uq, Ib, Ib2 (F2 0F 78 /r ib ib) — insert the source's low
    /// `Ib` bits into the destination's low quadword at bit `Ib2`.
    pub(super) fn insertq_vdq_uq_ib_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let destination = self.read_xmm_reg(instr.dst()).xmm64u(0);
        let source = self.read_xmm_reg(instr.src1()).xmm64u(0);
        let inserted = xmm_insertq(destination, source, instr.ib2(), instr.ib());
        self.write_xmm_lo_qword(instr.dst(), inserted);
        Ok(())
    }

    /// INSERTQ Vdq, Udq (F2 0F 79 /r) — as above, with the length in byte 8
    /// and the position in byte 9 of the source register.
    pub(super) fn insertq_vdq_udq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let source = self.read_xmm_reg(instr.src1());
        let destination = self.read_xmm_reg(instr.dst()).xmm64u(0);
        let inserted =
            xmm_insertq(destination, source.xmm64u(0), source.xmmubyte(9), source.xmmubyte(8));
        self.write_xmm_lo_qword(instr.dst(), inserted);
        Ok(())
    }

    // ========================================================================
    // Shuffle (PSHUFD, PSHUFHW, PSHUFLW) — SSE2
    // ========================================================================

    /// PSHUFD VdqWdqIb — shuffle dwords by imm8
    /// Each 2-bit field in imm8 selects one of the 4 source dwords.
    pub(super) fn pshufd_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_read_op2_xmm(instr)?;
        let order = instr.ib();

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32u(0, op.xmm32u((order & 3) as usize));
        result.set_xmm32u(1, op.xmm32u(((order >> 2) & 3) as usize));
        result.set_xmm32u(2, op.xmm32u(((order >> 4) & 3) as usize));
        result.set_xmm32u(3, op.xmm32u(((order >> 6) & 3) as usize));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSHUFHW VdqWdqIb — shuffle high words by imm8
    /// Low qword is copied unchanged; high 4 words are shuffled by imm8.
    pub(super) fn pshufhw_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_read_op2_xmm(instr)?;
        let order = instr.ib();

        let mut result = BxPackedXmmRegister::default();
        // Copy low qword unchanged
        result.set_xmm64u(0, op.xmm64u(0));
        // Shuffle high 4 words (indices 4-7) using imm8
        result.set_xmm16u(4, op.xmm16u(4 + (order & 3) as usize));
        result.set_xmm16u(5, op.xmm16u(4 + ((order >> 2) & 3) as usize));
        result.set_xmm16u(6, op.xmm16u(4 + ((order >> 4) & 3) as usize));
        result.set_xmm16u(7, op.xmm16u(4 + ((order >> 6) & 3) as usize));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSHUFLW VdqWdqIb — shuffle low words by imm8
    /// High qword is copied unchanged; low 4 words are shuffled by imm8.
    pub(super) fn pshuflw_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_read_op2_xmm(instr)?;
        let order = instr.ib();

        let mut result = BxPackedXmmRegister::default();
        // Shuffle low 4 words (indices 0-3) using imm8
        result.set_xmm16u(0, op.xmm16u((order & 3) as usize));
        result.set_xmm16u(1, op.xmm16u(((order >> 2) & 3) as usize));
        result.set_xmm16u(2, op.xmm16u(((order >> 4) & 3) as usize));
        result.set_xmm16u(3, op.xmm16u(((order >> 6) & 3) as usize));
        // Copy high qword unchanged
        result.set_xmm64u(1, op.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Insert/Extract (PINSRW, PEXTRW) — SSE2 XMM forms
    // ========================================================================

    /// PINSRW VdqEwIb — insert word at position specified by imm8 & 7
    pub(super) fn pinsrw_vdq_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src1().into())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
        };

        op1.set_xmm16u((instr.ib() & 7) as usize, op2);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PEXTRW GdUdqIb — extract word at position specified by imm8 & 7 to GPR32
    pub(super) fn pextrw_gd_udq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = op.xmm16u((instr.ib() & 7) as usize) as u32;
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    // ========================================================================
    // SSE4.1 Insert/Extract (PEXTRB/D/Q, PINSRB/D/Q)
    // ========================================================================

    // The extracts below are the handlers of the legacy, VEX and EVEX forms
    // alike, as in Bochs, whose def entries all lead with the GPR or memory
    // destination (OP_Ed/OP_Eq/OP_Mb/OP_Mw, ModRM.rm) and then the XMM source
    // (OP_Vdq/OP_Vps, ModRM.reg): `src1()` is the XMM source and, in a
    // register form, `dst()` the GPR destination — Bochs's `i->src()` and
    // `i->dst()`.

    /// PEXTRB EdVdqIbR — extract byte from XMM at imm8 & 0xF position to GPR32
    /// (register form). Bochs sse.cc `PEXTRB_EdVdqIbR`.
    pub(super) fn pextrb_ed_vdq_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = op.xmmubyte((instr.ib() & 0xF) as usize) as u32;
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// PEXTRB MbVdqIbM — extract byte from XMM at imm8 & 0xF position to memory
    /// (memory form). Bochs sse.cc `PEXTRB_MbVdqIbM`.
    pub(super) fn pextrb_mb_vdq_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = op.xmmubyte((instr.ib() & 0xF) as usize);
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        self.v_write_byte(seg, eaddr, result)?;
        Ok(())
    }

    /// PEXTRW EdVdqIbR — extract word from XMM at imm8 & 7 to a GPR
    /// (66 0F 3A 15 /r ib, register destination). Bochs `PEXTRW_EdVdqIbR`
    /// zero-extends the word into the full 32-bit register.
    pub(super) fn pextrw_ed_vdq_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = u32::from(op.xmm16u((instr.ib() & 0x7) as usize));
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// PEXTRW MwVdqIbM — extract word from XMM at imm8 & 7 to memory
    /// (66 0F 3A 15 /r ib, memory destination). Bochs `PEXTRW_MwVdqIbM`.
    pub(super) fn pextrw_mw_vdq_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = op.xmm16u((instr.ib() & 0x7) as usize);
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        self.v_write_word(seg, eaddr, result)?;
        Ok(())
    }

    /// PEXTRD EdVdqIb — extract dword from XMM at imm8 & 3 position (combined
    /// R/M form). Bochs sse.cc `PEXTRD_EdVdqIbR` / `PEXTRD_EdVdqIbM`, which
    /// EXTRACTPS shares.
    pub(super) fn pextrd_ed_vdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = op.xmm32u((instr.ib() & 3) as usize);
        if instr.mod_c0() {
            self.set_gpr32(instr.dst().into(), result);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_dword(seg, eaddr, result)?;
        }
        Ok(())
    }

    /// PEXTRQ EqVdqIb — extract qword from XMM at imm8 & 1 position (combined
    /// R/M form). Bochs sse.cc `PEXTRQ_EqVdqIbR` / `PEXTRQ_EqVdqIbM`.
    pub(super) fn pextrq_eq_vdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let result = op.xmm64u((instr.ib() & 1) as usize);
        if instr.mod_c0() {
            self.set_gpr64(instr.dst().into(), result);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_qword(seg, eaddr, result)?;
        }
        Ok(())
    }

    /// PINSRB VdqEbIb — insert byte from GPR/memory into XMM at imm8 & 0xF position (combined R/M)
    pub(super) fn pinsrb_vdq_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = if instr.mod_c0() {
            // BX_READ_8BIT_REGL — always low byte, never AH/CH/DH/BH
            self.gen_reg[instr.src1() as usize].rl()
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_byte(seg, eaddr)?
        };
        op1.set_xmmubyte((instr.ib() & 0xF) as usize, op2);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PINSRD VdqEdIb — insert dword from GPR/memory into XMM at imm8 & 3 position (combined R/M)
    pub(super) fn pinsrd_vdq_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_dword(seg, eaddr)?
        };
        op1.set_xmm32u((instr.ib() & 3) as usize, op2);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PINSRQ VdqEqIb — insert qword from GPR/memory into XMM at imm8 & 1 position (combined R/M)
    pub(super) fn pinsrq_vdq_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1().into())
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_qword(seg, eaddr)?
        };
        op1.set_xmm64u((instr.ib() & 1) as usize, op2);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    // ========================================================================
    // Min/Max/Average (PMINUB, PMAXUB, PMINSW, PMAXSW, PAVGB, PAVGW)
    // ========================================================================

    /// PMINUB VdqWdq — packed minimum unsigned bytes
    pub(super) fn pminub_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(i, op1.xmmubyte(i).min(op2.xmmubyte(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMAXUB VdqWdq — packed maximum unsigned bytes
    pub(super) fn pmaxub_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
            result.set_xmmubyte(i, op1.xmmubyte(i).max(op2.xmmubyte(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMINSW VdqWdq — packed minimum signed words
    pub(super) fn pminsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16s(i, op1.xmm16s(i).min(op2.xmm16s(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMAXSW VdqWdq — packed maximum signed words
    pub(super) fn pmaxsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
            result.set_xmm16s(i, op1.xmm16s(i).max(op2.xmm16s(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PAVGB VdqWdq — packed average unsigned bytes: (a + b + 1) >> 1
    pub(super) fn pavgb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pavgb)
    }

    /// PAVGW VdqWdq — packed average unsigned words: (a + b + 1) >> 1
    pub(super) fn pavgw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pavgw)
    }

    /// PMINSB VdqWdq (66 0F 38 38) — packed minimum signed bytes (SSE4.1)
    pub(super) fn pminsb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pminsb)
    }

    /// PMINSD VdqWdq (66 0F 38 39) — packed minimum signed dwords (SSE4.1)
    pub(super) fn pminsd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pminsd)
    }

    /// PMINUW VdqWdq (66 0F 38 3A) — packed minimum unsigned words (SSE4.1)
    pub(super) fn pminuw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pminuw)
    }

    /// PMAXSB VdqWdq (66 0F 38 3C) — packed maximum signed bytes (SSE4.1)
    pub(super) fn pmaxsb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pmaxsb)
    }

    /// PMAXSD VdqWdq (66 0F 38 3D) — packed maximum signed dwords (SSE4.1)
    pub(super) fn pmaxsd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pmaxsd)
    }

    /// PMAXUW VdqWdq (66 0F 38 3E) — packed maximum unsigned words (SSE4.1)
    pub(super) fn pmaxuw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pmaxuw)
    }

    /// PCMPEQQ VdqWdq (66 0F 38 29) — packed compare equal qwords (SSE4.1)
    pub(super) fn pcmpeqq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pcmpeqq)
    }

    /// PCMPGTQ VdqWdq (66 0F 38 37) — packed compare signed greater-than
    /// qwords (SSE4.2)
    pub(super) fn pcmpgtq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pcmpgtq)
    }

    // ========================================================================
    // Misc (PMOVMSKB, PSADBW, MASKMOVDQU)
    // ========================================================================

    /// PMOVMSKB GdUdq — move byte mask: collect sign bits of 16 bytes into GPR32
    pub(super) fn pmovmskb_gd_udq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.src1());
        let mut mask = 0u32;
        for i in 0..16 {
            if op.xmmubyte(i) & 0x80 != 0 {
                mask |= 1 << i;
            }
        }
        self.set_gpr32(instr.dst().into(), mask);
        Ok(())
    }

    /// PSADBW VdqWdq — sum of absolute differences
    /// Computes SAD for low 8 bytes -> result qword 0, high 8 bytes -> result qword 1.
    pub(super) fn psadbw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        let mut temp0 = 0u16;
        for i in 0..8 {
            temp0 += (op1.xmmubyte(i) as i16 - op2.xmmubyte(i) as i16).unsigned_abs();
        }
        result.set_xmm64u(0, temp0 as u64);

        let mut temp1 = 0u16;
        for i in 8..16 {
            temp1 += (op1.xmmubyte(i) as i16 - op2.xmmubyte(i) as i16).unsigned_abs();
        }
        result.set_xmm64u(1, temp1 as u64);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MASKMOVDQU VdqUdq — masked store bytes using DS:EDI
    /// For each byte where mask bit 7 is set, store the corresponding byte
    /// from the source XMM register to memory at [DS:(E/R)DI].
    /// Bochs: sse_move.cc MASKMOVDQU_VdqUdq
    pub(super) fn maskmovdqu_vdq_udq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;

        let op = self.read_xmm_reg(instr.dst()); // nnn = Vdq (data source)
        let mask = self.read_xmm_reg(instr.src1()); // rm = Udq (mask)

        // Bochs: bx_address rdi = RDI & i->asize_mask();
        const ASIZE_MASK: [u64; 4] = [
            0xFFFF,
            0xFFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFF,
        ];
        let asize = (instr.as32_l() != 0) as usize | (((instr.as64_l() != 0) as usize) << 1);
        let rdi = self.rdi() & ASIZE_MASK[asize];

        // Bochs: i->seg() — allow segment override prefixes
        let seg = BxSegregs::from(instr.seg());

        // Bochs reads the full 16 bytes BEFORE checking the mask to ensure
        // page fault even if mask is all zeros (sse_move.cc)
        let mut temp = super::xmm::BxPackedXmmRegister::default();
        temp.set_xmm64u(0, self.v_read_qword(seg, rdi)?);
        temp.set_xmm64u(
            1,
            self.v_read_qword(seg, (rdi.wrapping_add(8)) & ASIZE_MASK[asize])?,
        );

        // No data will be written to memory if mask is all 0s (Bochs sse_move.cc)
        let any_set = (mask.xmm64u(0) | mask.xmm64u(1)) & 0x8080808080808080 != 0;
        if !any_set {
            return Ok(());
        }

        // Merge masked bytes into temp
        for j in 0..16usize {
            if mask.xmmubyte(j) & 0x80 != 0 {
                temp.set_xmmubyte(j, op.xmmubyte(j));
            }
        }

        // Write result back to memory (Bochs sse_move.cc)
        self.v_write_qword(
            seg,
            (rdi.wrapping_add(8)) & ASIZE_MASK[asize],
            temp.xmm64u(1),
        )?;
        self.v_write_qword(seg, rdi, temp.xmm64u(0))?;
        Ok(())
    }

    // ========================================================================
    // SSSE3 128-bit packed integer (matching Bochs sse.cc / simd_int.h)
    // ========================================================================

    /// PSHUFB VdqWdq (66 0F 38 00) - Packed Shuffle Bytes (128-bit)
    /// Bochs: PSHUFB_VdqWdqR / xmm_pshufb (simd_int.h)
    pub(super) fn pshufb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for n in 0..16usize {
            let mask = op2.xmmubyte(n);
            if mask & 0x80 != 0 {
                result.set_xmmubyte(n, 0);
            } else {
                result.set_xmmubyte(n, op1.xmmubyte((mask & 0xf) as usize));
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMADDUBSW VdqWdq (66 0F 38 04) - Multiply Unsigned/Signed Bytes, Add Pairs (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_pmaddubsw> / simd_int.h
    pub(super) fn pmaddubsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_pmaddubsw)
    }

    /// PSIGNB VdqWdq (66 0F 38 08) - Negate/Zero/Keep Bytes Based on Sign (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_psignb> / simd_int.h
    pub(super) fn psignb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_psignb)
    }

    /// PSIGNW VdqWdq (66 0F 38 09) - Negate/Zero/Keep Words Based on Sign (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_psignw> / simd_int.h
    pub(super) fn psignw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_psignw)
    }

    /// PSIGND VdqWdq (66 0F 38 0A) - Negate/Zero/Keep Dwords Based on Sign (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_psignd> / simd_int.h
    pub(super) fn psignd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_psignd)
    }

    /// PHADDW VdqWdq (66 0F 38 01) - Horizontal Add Words (Bochs
    /// HANDLE_SSE_2OP<xmm_phaddw>)
    pub(super) fn phaddw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_phaddw)
    }

    /// PHADDD VdqWdq (66 0F 38 02) - Horizontal Add Dwords
    pub(super) fn phaddd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_phaddd)
    }

    /// PHADDSW VdqWdq (66 0F 38 03) - Horizontal Add Words with Signed
    /// Saturation
    pub(super) fn phaddsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_phaddsw)
    }

    /// PHSUBW VdqWdq (66 0F 38 05) - Horizontal Subtract Words
    pub(super) fn phsubw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_phsubw)
    }

    /// PHSUBD VdqWdq (66 0F 38 06) - Horizontal Subtract Dwords
    pub(super) fn phsubd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_phsubd)
    }

    /// PHSUBSW VdqWdq (66 0F 38 07) - Horizontal Subtract Words with Signed
    /// Saturation
    pub(super) fn phsubsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_2op(instr, simd_int::xmm_phsubsw)
    }

    /// PALIGNR VdqWdqIb (66 0F 3A 0F) - Packed Align Right (128-bit)
    /// Bochs: PALIGNR_VdqWdqIbR / xmm_palignr (simd_int.h)
    pub(super) fn palignr_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let shift = instr.ib();

        // result = [op1:op2] >> (shift * 8)
        // op1 is high, op2 is low in the concatenated 256-bit value
        let mut result = op2;
        if shift >= 32 {
            // All zeros
            result = BxPackedXmmRegister::default();
        } else if shift >= 16 {
            // Only op1 bits remain, shifted right
            result = op1;
            let bit_shift = ((shift - 16) as u64) * 8;
            if bit_shift >= 128 {
                result = BxPackedXmmRegister::default();
            } else if bit_shift >= 64 {
                let s = bit_shift - 64;
                result.set_xmm64u(0, if s < 64 { result.xmm64u(1) >> s } else { 0 });
                result.set_xmm64u(1, 0);
            } else if bit_shift > 0 {
                result.set_xmm64u(
                    0,
                    (result.xmm64u(0) >> bit_shift) | (result.xmm64u(1) << (64 - bit_shift)),
                );
                result.set_xmm64u(1, result.xmm64u(1) >> bit_shift);
            }
        } else if shift > 0 {
            let bit_shift = (shift as u64) * 8;
            if bit_shift > 64 {
                let s = bit_shift - 64;
                result.set_xmm64u(0, (op2.xmm64u(1) >> s) | (op1.xmm64u(0) << (64 - s)));
                result.set_xmm64u(1, (op1.xmm64u(0) >> s) | (op1.xmm64u(1) << (64 - s)));
            } else if bit_shift == 64 {
                result.set_xmm64u(0, op2.xmm64u(1));
                result.set_xmm64u(1, op1.xmm64u(0));
            } else {
                // bit_shift < 64 and > 0
                result.set_xmm64u(
                    0,
                    (op2.xmm64u(0) >> bit_shift) | (op2.xmm64u(1) << (64 - bit_shift)),
                );
                result.set_xmm64u(
                    1,
                    (op2.xmm64u(1) >> bit_shift) | (op1.xmm64u(0) << (64 - bit_shift)),
                );
            }
        }
        // shift == 0: result = op2 (already set)

        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // SSE4.1 128-bit packed integer (matching Bochs sse.cc / simd_int.h)
    // ========================================================================

    /// PBLENDVB VdqWdq (66 0F 38 10) - Variable Blend Packed Bytes
    /// Bochs: PBLENDVB_VdqWdqR / xmm_pblendvb (simd_int.h)
    /// Implicit mask register: XMM0
    pub(super) fn pblendvb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let mask = self.read_xmm_reg(0); // XMM0 is implicit mask

        pblendvb_lane(&mut op1, &op2, &mask);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PABSB VdqWdq (66 0F 38 1C) — Packed Absolute Value Bytes
    /// Bochs: HANDLE_SSE_1OP<xmm_pabsb> (simd_int.h)
    pub(super) fn pabsb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_read_op2_xmm(instr)?;
        pabsb_lane(&mut op);
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PABSW VdqWdq (66 0F 38 1D) — Packed Absolute Value Words
    /// Bochs: HANDLE_SSE_1OP<xmm_pabsw> (simd_int.h)
    pub(super) fn pabsw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_read_op2_xmm(instr)?;
        pabsw_lane(&mut op);
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// PABSD VdqWdq (66 0F 38 1E) — Packed Absolute Value Dwords
    /// Bochs: HANDLE_SSE_1OP<xmm_pabsd> (simd_int.h)
    pub(super) fn pabsd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_read_op2_xmm(instr)?;
        pabsd_lane(&mut op);
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// MPSADBW VdqWdqIb (66 0F 3A 42) — Multiple Sums of Absolute Differences
    /// Bochs: MPSADBW_VdqWdqIbR via simd_int.h xmm_mpsadbw
    pub(super) fn mpsadbw_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let result = mpsadbw_lane(&op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PHMINPOSUW VdqWdq (66 0F 38 41) — Horizontal Minimum of Unsigned Words
    /// Bochs: PHMINPOSUW_VdqWdqR (sse.cc)
    pub(super) fn phminposuw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_read_op2_xmm(instr)?;
        let result = phminposuw_core(&op);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// INSERTPS VpsWssIb (66 0F 3A 21) — Insert Packed Single Precision
    /// Bochs: INSERTPS_VpsWssIbR / INSERTPS_VpsWssIbM (sse.cc)
    pub(super) fn insertps_vps_wss_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let control = instr.ib();
        let op2 = if instr.mod_c0() {
            // Register form: imm[7:6] selects the source dword
            self.read_xmm_reg(instr.src1())
                .xmm32u(((control >> 6) & 3) as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)?
        };
        let mut op1 = self.read_xmm_reg(instr.dst());
        insertps_core(&mut op1, op2, control);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PTEST VdqWdq (66 0F 38 17) - Logical Compare
    /// Bochs: PTEST_VdqWdqR (sse.cc)
    /// Sets ZF if (op2 AND op1) == 0, CF if (op2 AND NOT op1) == 0
    pub(super) fn ptest_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        // Bochs sse.cc PTEST_VdqWdqR: clearEFlagsOSZAPC();
        self.oszapc.set_oszapc_logic_32(1);
        if (op2.xmm64u(0) & op1.xmm64u(0)) == 0 && (op2.xmm64u(1) & op1.xmm64u(1)) == 0 {
            self.oszapc.set_zf(true);
        }
        if (op2.xmm64u(0) & !op1.xmm64u(0)) == 0 && (op2.xmm64u(1) & !op1.xmm64u(1)) == 0 {
            self.oszapc.set_cf(true);
        }
        Ok(())
    }

    /// PMULDQ VdqWdq (66 0F 38 28) - Multiply Packed Signed Dword to Qword
    /// Bochs: HANDLE_SSE_2OP<xmm_pmuldq> / simd_int.h
    pub(super) fn pmuldq_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64s(0, (op1.xmm32s(0) as i64) * (op2.xmm32s(0) as i64));
        result.set_xmm64s(1, (op1.xmm32s(2) as i64) * (op2.xmm32s(2) as i64));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PMINUD VdqWdq (66 0F 38 3B) - Minimum of Packed Unsigned Dwords
    /// Bochs: HANDLE_SSE_2OP<xmm_pminud> / simd_int.h
    pub(super) fn pminud_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        for n in 0..4usize {
            if op2.xmm32u(n) < op1.xmm32u(n) {
                op1.set_xmm32u(n, op2.xmm32u(n));
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PMAXUD VdqWdq (66 0F 38 3F) - Maximum of Packed Unsigned Dwords
    /// Bochs: HANDLE_SSE_2OP<xmm_pmaxud> / simd_int.h
    pub(super) fn pmaxud_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        for n in 0..4usize {
            if op2.xmm32u(n) > op1.xmm32u(n) {
                op1.set_xmm32u(n, op2.xmm32u(n));
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PMULLD VdqWdq (66 0F 38 40) - Multiply Packed Signed Dword, Low Result
    /// Bochs: HANDLE_SSE_2OP<xmm_pmulld> / simd_int.h
    pub(super) fn pmulld_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        for n in 0..4usize {
            op1.set_xmm32s(n, op1.xmm32s(n).wrapping_mul(op2.xmm32s(n)));
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// PBLENDW VdqWdqIb (66 0F 3A 0E) - Blend Packed Words
    /// Bochs: PBLENDW_VdqWdqIbR / xmm_pblendw (simd_int.h)
    pub(super) fn pblendw_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let mut mask = instr.ib() as u32;

        for n in 0..8usize {
            if mask & 1 != 0 {
                op1.set_xmm16u(n, op2.xmm16u(n));
            }
            mask >>= 1;
        }
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// BLENDPS VpsWpsIb (66 0F 3A 0C) - Blend Packed Single-FP by immediate
    /// Bochs: BLENDPS_VpsWpsIbR / xmm_blendps (simd_int.h)
    pub(super) fn blendps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        blendps_lane(&mut op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// BLENDPD VpdWpdIb (66 0F 3A 0D) - Blend Packed Double-FP by immediate
    /// Bochs: BLENDPD_VpdWpdIbR / xmm_blendpd (simd_int.h)
    pub(super) fn blendpd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        blendpd_lane(&mut op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// BLENDVPS VpsWps (66 0F 38 14) - Variable Blend Packed Single-FP
    /// Bochs: BLENDVPS_VpsWpsR / xmm_blendvps (simd_int.h)
    /// Implicit mask register: XMM0 (sign bit of each dword lane)
    pub(super) fn blendvps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let mask = self.read_xmm_reg(0); // XMM0 is implicit mask
        blendvps_lane(&mut op1, &op2, &mask);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// BLENDVPD VpdWpd (66 0F 38 15) - Variable Blend Packed Double-FP
    /// Bochs: BLENDVPD_VpdWpdR / xmm_blendvpd (simd_int.h)
    /// Implicit mask register: XMM0 (sign bit of each qword lane)
    pub(super) fn blendvpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;
        let mask = self.read_xmm_reg(0); // XMM0 is implicit mask
        blendvpd_lane(&mut op1, &op2, &mask);
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }
}

/// SSE4A, run on the AMD model in flat long mode. The public API builds every
/// machine on the default (Intel) model, so these drive the AMD processor
/// through `TestMachine` rather than as an integration test. They rely on
/// 64-bit mode, flat segment caches, an identity map of the low 2 MiB and
/// CR4.OSFXSR — not on a GDT, a wider map or the RFLAGS a guest is handed.
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::cpu::api_bridge::SegmentSize;
    use crate::cpu::exec_ctx::{ExecCtx, TestMachine};
    use crate::cpu::instrumentation::X86Reg;
    use crate::memory::BxMemC;

    /// Where the paging structures go: one PML4, one PDPT and one page
    /// directory whose first entry maps the low 2 MiB onto itself.
    const PML4: u64 = 0x1000;
    const PDPT: u64 = 0x2000;
    const PAGE_DIRECTORY: u64 = 0x3000;
    /// Present and writable; and for a page-directory entry, 2 MiB.
    const PRESENT_WRITABLE: u64 = 0x3;
    const LARGE_PAGE: u64 = 0x80;
    /// CR4.OSFXSR: legacy SSE executes.
    const CR4_OSFXSR: u32 = 1 << 9;
    const CODE_SELECTOR: u16 = 0x08;
    const DATA_SELECTOR: u16 = 0x10;

    /// The AMD model: the one that advertises SSE4A.
    fn ryzen() -> TestMachine {
        TestMachine::with_model(crate::cpu::CpuModel::amd_ryzen())
    }

    /// Write `bytes` to RAM at `at`, all of them.
    fn write_ram(memory: &mut BxMemC, at: u64, bytes: &[u8]) {
        let copied = memory.write_ram(at, bytes).expect("RAM");
        assert_eq!(copied, bytes.len(), "the write at {at:#x} reached RAM whole");
    }

    /// Read `out.len()` bytes of RAM at `at`, all of them.
    fn read_ram(memory: &mut BxMemC, at: u64, out: &mut [u8]) {
        let copied = memory.read_ram(at, out).expect("RAM");
        assert_eq!(copied, out.len(), "the read at {at:#x} came from RAM whole");
    }

    /// The processor from reset in flat long mode with CR4.OSFXSR set: a
    /// 64-bit code segment cache, flat data segment caches, CR0.PG, CR4.PAE
    /// and EFER.LMA over an identity map of the low 2 MiB. No GDT backs the
    /// selectors, so nothing here may reload a segment register.
    fn long_mode_sse(machine: &mut TestMachine) -> ExecCtx<'_, ()> {
        let memory = machine.memory_mut();
        write_ram(memory, PML4, &(PDPT | PRESENT_WRITABLE).to_le_bytes());
        write_ram(memory, PDPT, &(PAGE_DIRECTORY | PRESENT_WRITABLE).to_le_bytes());
        write_ram(memory, PAGE_DIRECTORY, &(PRESENT_WRITABLE | LARGE_PAGE).to_le_bytes());

        let mut cpu = machine.ctx();
        cpu.reset(crate::cpu::ResetReason::Hardware);
        cpu.set_seg_for_api(X86Reg::Cs, CODE_SELECTOR, 0, 0xFFFF_FFFF, SegmentSize::Long64);
        for reg in [X86Reg::Ds, X86Reg::Es, X86Reg::Ss, X86Reg::Fs, X86Reg::Gs] {
            cpu.set_seg_for_api(reg, DATA_SELECTOR, 0, 0xFFFF_FFFF, SegmentSize::Bits32);
        }
        cpu.enter_long_mode_for_api(PML4);
        assert!(cpu.long64_mode(), "the processor is in 64-bit mode");
        let cr4 = cpu.cr4.get32();
        cpu.set_cr4_raw_for_api(cr4 | CR4_OSFXSR);
        cpu
    }

    /// Decode `bytes` as one instruction and execute it; it must retire. A
    /// fault would unwind as `CpuLoopRestart`, so that fails the test too.
    fn execute(cpu: &mut ExecCtx<'_, ()>, bytes: &[u8]) {
        let instr = rusty_box_decoder::fetch_decode64(bytes).expect("decodes");
        if let Err(error) = cpu.execute_instruction(&instr) {
            panic!("{:?} did not retire: {error:?}", instr.get_ia_opcode());
        }
    }

    fn xmm_from_qwords(low: u64, high: u64) -> BxPackedXmmRegister {
        let mut reg = BxPackedXmmRegister::default();
        reg.set_xmm64u(0, low);
        reg.set_xmm64u(1, high);
        reg
    }

    const FIELD: u64 = 0x1234_5678_9ABC_DEF0;
    const KEPT: u64 = 0xAAAA_BBBB_CCCC_DDDD;

    /// EXTRQ moves the field it names to bit 0 and keeps the high quadword;
    /// a length of 0 means all 64 bits (Bochs sse.cc `xmm_extrq`).
    #[test]
    fn extrq_extracts_the_field_it_names() {
        let mut machine = ryzen();
        let mut cpu = long_mode_sse(&mut machine);
        cpu.write_xmm_reg(1, xmm_from_qwords(FIELD, KEPT));
        // extrq xmm1, 8, 4: 8 bits at bit 4.
        execute(&mut cpu, &[0x66, 0x0F, 0x78, 0xC1, 0x08, 0x04]);
        assert_eq!(cpu.read_xmm_reg(1).xmm64u(0), 0xEF);
        assert_eq!(cpu.read_xmm_reg(1).xmm64u(1), KEPT);

        // extrq xmm1, xmm2 with length 16 in bits 5:0 and position 8 in 13:8.
        cpu.write_xmm_reg(1, xmm_from_qwords(FIELD, KEPT));
        cpu.write_xmm_reg(2, xmm_from_qwords(0x0810, 0));
        execute(&mut cpu, &[0x66, 0x0F, 0x79, 0xCA]);
        assert_eq!(cpu.read_xmm_reg(1).xmm64u(0), 0xBCDE);

        // Length 0, position 12: everything from bit 12 up.
        cpu.write_xmm_reg(1, xmm_from_qwords(FIELD, KEPT));
        execute(&mut cpu, &[0x66, 0x0F, 0x78, 0xC1, 0x00, 0x0C]);
        assert_eq!(cpu.read_xmm_reg(1).xmm64u(0), FIELD >> 12);
    }

    /// INSERTQ replaces the field it names with the source's low bits.
    #[test]
    fn insertq_replaces_the_field_it_names() {
        let mut machine = ryzen();
        let mut cpu = long_mode_sse(&mut machine);
        cpu.write_xmm_reg(0, xmm_from_qwords(u64::MAX, KEPT));
        cpu.write_xmm_reg(1, xmm_from_qwords(0x1234, 0));
        // insertq xmm0, xmm1, 8, 16: 8 bits at bit 16.
        execute(&mut cpu, &[0xF2, 0x0F, 0x78, 0xC1, 0x08, 0x10]);
        assert_eq!(cpu.read_xmm_reg(0).xmm64u(0), 0xFFFF_FFFF_FF34_FFFF);
        assert_eq!(cpu.read_xmm_reg(0).xmm64u(1), KEPT);

        // insertq xmm0, xmm1 with length 4 in byte 8 and position 60 in byte 9.
        cpu.write_xmm_reg(0, xmm_from_qwords(0, KEPT));
        cpu.write_xmm_reg(1, xmm_from_qwords(0xF, 0x3C04));
        execute(&mut cpu, &[0xF2, 0x0F, 0x79, 0xC1]);
        assert_eq!(cpu.read_xmm_reg(0).xmm64u(0), 0xF000_0000_0000_0000);
    }

    /// MOVNTSS and MOVNTSD store the low dword and qword, through the page
    /// walk like any guest store.
    #[test]
    fn the_scalar_non_temporal_stores_write_the_low_element() {
        const AT: u64 = 0x8000;
        let mut machine = ryzen();
        let mut cpu = long_mode_sse(&mut machine);
        cpu.write_xmm_reg(3, xmm_from_qwords(FIELD, KEPT));
        write_ram(cpu.memory, AT, &[0u8; 16]);
        // movntss [0x8000], xmm3: F3 0F 2B /r, ModRM 0x1C + SIB 0x25 disp32.
        execute(&mut cpu, &[0xF3, 0x0F, 0x2B, 0x1C, 0x25, 0x00, 0x80, 0x00, 0x00]);
        // movntsd [0x8008], xmm3
        execute(&mut cpu, &[0xF2, 0x0F, 0x2B, 0x1C, 0x25, 0x08, 0x80, 0x00, 0x00]);
        let mut stored = [0u8; 16];
        read_ram(cpu.memory, AT, &mut stored);
        assert_eq!(stored[..8], [0xF0, 0xDE, 0xBC, 0x9A, 0, 0, 0, 0], "the low dword alone");
        assert_eq!(stored[8..], FIELD.to_le_bytes(), "the low qword");
    }
}
