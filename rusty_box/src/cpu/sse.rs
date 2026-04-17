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
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// Saturation helpers (matching Bochs sse_int.cc / mmx.cc inline functions)
// ============================================================================

/// Saturate a signed 16-bit value to signed 8-bit range [-128, 127]
#[inline]
fn saturate_word_s_to_byte_s(val: i16) -> i8 {
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
fn saturate_word_s_to_byte_u(val: i16) -> u8 {
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
fn saturate_dword_s_to_word_s(val: i32) -> i16 {
    if val > 32767 {
        32767
    } else if val < -32768 {
        -32768
    } else {
        val as i16
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // SSE helper: read op2 (register or memory)
    // ========================================================================

    /// Read the second operand for SSE packed integer instructions.
    /// If mod_c0, reads an XMM register; otherwise reads 128 bits from memory.
    #[inline]
    pub(super) fn sse_read_op2_xmm(&mut self, instr: &Instruction) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_xmmword(seg, eaddr)
        }
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
                result.set_xmm_sbyte(i, saturate_word_s_to_byte_s(op1.xmm_sbyte(i) as i16 + op2.xmm_sbyte(i) as i16));
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
                result.set_xmm16s(i, saturate_dword_s_to_word_s(op1.xmm16s(i) as i32 + op2.xmm16s(i) as i32));
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
                result.set_xmm_sbyte(i, saturate_word_s_to_byte_s(op1.xmm_sbyte(i) as i16 - op2.xmm_sbyte(i) as i16));
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
                result.set_xmm16s(i, saturate_dword_s_to_word_s(op1.xmm16s(i) as i32 - op2.xmm16s(i) as i32));
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
                result.set_xmm16u(i, (op1.xmm16u(i) as u32).wrapping_mul(op2.xmm16u(i) as u32) as u16);
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
                result.set_xmm16u(i, ((op1.xmm16s(i) as i32 * op2.xmm16s(i) as i32) >> 16) as u16);
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
                result.set_xmm16u(i, ((op1.xmm16u(i) as u32 * op2.xmm16u(i) as u32) >> 16) as u16);
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
    /// For each pair of adjacent words: result[i] = op1[2i]*op2[2i] + op1[2i+1]*op2[2i+1]
    /// With the 0x80008000 overflow guard matching Bochs.
    pub(super) fn pmaddwd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                if op1.xmm16u(i * 2) == 0x8000
                    && op1.xmm16u(i * 2 + 1) == 0x8000
                    && op2.xmm16u(i * 2) == 0x8000
                    && op2.xmm16u(i * 2 + 1) == 0x8000
                {
                    result.set_xmm32u(i, 0x80000000);
                } else {
                    result.set_xmm32s(i, (op1.xmm16s(i * 2) as i32) * (op2.xmm16s(i * 2) as i32)
                        + (op1.xmm16s(i * 2 + 1) as i32) * (op2.xmm16s(i * 2 + 1) as i32));
                }
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
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
                result.set_xmmubyte(i, if op1.xmmubyte(i) == op2.xmmubyte(i) {
                    0xff
                } else {
                    0
                });
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
                result.set_xmm16u(i, if op1.xmm16u(i) == op2.xmm16u(i) {
                    0xffff
                } else {
                    0
                });
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
                result.set_xmm32u(i, if op1.xmm32u(i) == op2.xmm32u(i) {
                    0xffffffff
                } else {
                    0
                });
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
                result.set_xmmubyte(i, if op1.xmm_sbyte(i) > op2.xmm_sbyte(i) {
                    0xff
                } else {
                    0
                });
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
                result.set_xmm16u(i, if op1.xmm16s(i) > op2.xmm16s(i) {
                    0xffff
                } else {
                    0
                });
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
                result.set_xmm32u(i, if op1.xmm32s(i) > op2.xmm32s(i) {
                    0xffffffff
                } else {
                    0
                });
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
        // Bochs simd_int.h uses `> 64` (note: count=64 is technically UB in C,
        // but we match Bochs behavior exactly here)
        if count > 64 {
            op1 = BxPackedXmmRegister::default();
        } else if count > 0 {
            let shift = count.min(63) as u32; // clamp to avoid Rust panic on >> 64
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
        // Bochs simd_int.h xmm_psllq uses `> 64` — match exactly
        if count > 64 {
            op1 = BxPackedXmmRegister::default();
        } else if count > 0 {
            let shift = count.min(63) as u32;
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

        // Bochs simd_int.h uses `shift > 64` for qword immediate shifts
        if shift > 64 {
            op = BxPackedXmmRegister::default();
        } else if shift > 0 {
            let s = (shift as u32).min(63);
                op.set_xmm64u(0, op.xmm64u(0) >> s as u64);
                op.set_xmm64u(1, op.xmm64u(1) >> s as u64);
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

        // Bochs simd_int.h uses `shift > 64` for qword immediate shifts
        if shift > 64 {
            op = BxPackedXmmRegister::default();
        } else if shift > 0 {
            let s = (shift as u32).min(63);
                op.set_xmm64u(0, op.xmm64u(0) << s as u64);
                op.set_xmm64u(1, op.xmm64u(1) << s as u64);
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
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
            result.set_xmm_sbyte(0, saturate_word_s_to_byte_s(op1.xmm16s(0)));
            result.set_xmm_sbyte(1, saturate_word_s_to_byte_s(op1.xmm16s(1)));
            result.set_xmm_sbyte(2, saturate_word_s_to_byte_s(op1.xmm16s(2)));
            result.set_xmm_sbyte(3, saturate_word_s_to_byte_s(op1.xmm16s(3)));
            result.set_xmm_sbyte(4, saturate_word_s_to_byte_s(op1.xmm16s(4)));
            result.set_xmm_sbyte(5, saturate_word_s_to_byte_s(op1.xmm16s(5)));
            result.set_xmm_sbyte(6, saturate_word_s_to_byte_s(op1.xmm16s(6)));
            result.set_xmm_sbyte(7, saturate_word_s_to_byte_s(op1.xmm16s(7)));
            result.set_xmm_sbyte(8, saturate_word_s_to_byte_s(op2.xmm16s(0)));
            result.set_xmm_sbyte(9, saturate_word_s_to_byte_s(op2.xmm16s(1)));
            result.set_xmm_sbyte(10, saturate_word_s_to_byte_s(op2.xmm16s(2)));
            result.set_xmm_sbyte(11, saturate_word_s_to_byte_s(op2.xmm16s(3)));
            result.set_xmm_sbyte(12, saturate_word_s_to_byte_s(op2.xmm16s(4)));
            result.set_xmm_sbyte(13, saturate_word_s_to_byte_s(op2.xmm16s(5)));
            result.set_xmm_sbyte(14, saturate_word_s_to_byte_s(op2.xmm16s(6)));
            result.set_xmm_sbyte(15, saturate_word_s_to_byte_s(op2.xmm16s(7)));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PACKSSDW VdqWdq — pack signed dwords to signed words with saturation
    pub(super) fn packssdw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
            result.set_xmm16s(0, saturate_dword_s_to_word_s(op1.xmm32s(0)));
            result.set_xmm16s(1, saturate_dword_s_to_word_s(op1.xmm32s(1)));
            result.set_xmm16s(2, saturate_dword_s_to_word_s(op1.xmm32s(2)));
            result.set_xmm16s(3, saturate_dword_s_to_word_s(op1.xmm32s(3)));
            result.set_xmm16s(4, saturate_dword_s_to_word_s(op2.xmm32s(0)));
            result.set_xmm16s(5, saturate_dword_s_to_word_s(op2.xmm32s(1)));
            result.set_xmm16s(6, saturate_dword_s_to_word_s(op2.xmm32s(2)));
            result.set_xmm16s(7, saturate_dword_s_to_word_s(op2.xmm32s(3)));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PACKUSWB VdqWdq — pack signed words to unsigned bytes with saturation
    pub(super) fn packuswb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
            result.set_xmmubyte(0, saturate_word_s_to_byte_u(op1.xmm16s(0)));
            result.set_xmmubyte(1, saturate_word_s_to_byte_u(op1.xmm16s(1)));
            result.set_xmmubyte(2, saturate_word_s_to_byte_u(op1.xmm16s(2)));
            result.set_xmmubyte(3, saturate_word_s_to_byte_u(op1.xmm16s(3)));
            result.set_xmmubyte(4, saturate_word_s_to_byte_u(op1.xmm16s(4)));
            result.set_xmmubyte(5, saturate_word_s_to_byte_u(op1.xmm16s(5)));
            result.set_xmmubyte(6, saturate_word_s_to_byte_u(op1.xmm16s(6)));
            result.set_xmmubyte(7, saturate_word_s_to_byte_u(op1.xmm16s(7)));
            result.set_xmmubyte(8, saturate_word_s_to_byte_u(op2.xmm16s(0)));
            result.set_xmmubyte(9, saturate_word_s_to_byte_u(op2.xmm16s(1)));
            result.set_xmmubyte(10, saturate_word_s_to_byte_u(op2.xmm16s(2)));
            result.set_xmmubyte(11, saturate_word_s_to_byte_u(op2.xmm16s(3)));
            result.set_xmmubyte(12, saturate_word_s_to_byte_u(op2.xmm16s(4)));
            result.set_xmmubyte(13, saturate_word_s_to_byte_u(op2.xmm16s(5)));
            result.set_xmmubyte(14, saturate_word_s_to_byte_u(op2.xmm16s(6)));
            result.set_xmmubyte(15, saturate_word_s_to_byte_u(op2.xmm16s(7)));
        self.write_xmm_reg_lo128(instr.dst(), result);
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

    /// PEXTRB EdVdqIbR — extract byte from XMM at imm8 & 0xF position to GPR32 (register form)
    /// Decoder: 0F 3A map → dst=nnn (XMM source), src1=rm (GPR destination)
    pub(super) fn pextrb_ed_vdq_ib_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.dst()); // nnn = XMM source
        let result = op.xmmubyte((instr.ib() & 0xF) as usize) as u32;
        self.set_gpr32(instr.src1().into(), result); // rm = GPR destination
        Ok(())
    }

    /// PEXTRB MbVdqIbM — extract byte from XMM at imm8 & 0xF position to memory (memory form)
    pub(super) fn pextrb_mb_vdq_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.dst()); // nnn = XMM source
        let result = op.xmmubyte((instr.ib() & 0xF) as usize);
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        self.v_write_byte(seg, eaddr, result)?;
        Ok(())
    }

    /// PEXTRD EdVdqIb — extract dword from XMM at imm8 & 3 position (combined R/M form)
    /// Decoder: 0F 3A map → dst=nnn (XMM source), src1=rm (GPR/mem destination)
    pub(super) fn pextrd_ed_vdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.dst()); // nnn = XMM source
        let result = op.xmm32u((instr.ib() & 3) as usize);
        if instr.mod_c0() {
            self.set_gpr32(instr.src1().into(), result); // rm = GPR destination
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_write_dword(seg, eaddr, result)?;
        }
        Ok(())
    }

    /// PEXTRQ EqVdqIb — extract qword from XMM at imm8 & 1 position (combined R/M form)
    /// Decoder: 0F 3A map → dst=nnn (XMM source), src1=rm (GPR/mem destination)
    pub(super) fn pextrq_eq_vdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.read_xmm_reg(instr.dst()); // nnn = XMM source
        let result = op.xmm64u((instr.ib() & 1) as usize);
        if instr.mod_c0() {
            self.set_gpr64(instr.src1().into(), result); // rm = GPR destination
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
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..16 {
                result.set_xmmubyte(i, ((op1.xmmubyte(i) as u16 + op2.xmmubyte(i) as u16 + 1) >> 1) as u8);
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PAVGW VdqWdq — packed average unsigned words: (a + b + 1) >> 1
    pub(super) fn pavgw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = BxPackedXmmRegister::default();
        for i in 0..8 {
                result.set_xmm16u(i, ((op1.xmm16u(i) as u32 + op2.xmm16u(i) as u32 + 1) >> 1) as u16);
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
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

        let op = self.read_xmm_reg(instr.dst());     // nnn = Vdq (data source)
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
            temp.set_xmm64u(1, self.v_read_qword(seg, (rdi.wrapping_add(8)) & ASIZE_MASK[asize])?);

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
            self.v_write_qword(seg, (rdi.wrapping_add(8)) & ASIZE_MASK[asize], temp.xmm64u(1))?;
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
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = op1;
        for n in 0..8usize {
                let temp = (op1.xmmubyte(n * 2) as i32) * (op2.xmm_sbyte(n * 2) as i32)
                    + (op1.xmmubyte(n * 2 + 1) as i32) * (op2.xmm_sbyte(n * 2 + 1) as i32);
                result.set_xmm16s(n, saturate_dword_s_to_word_s(temp));
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSIGNB VdqWdq (66 0F 38 08) - Negate/Zero/Keep Bytes Based on Sign (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_psignb> / simd_int.h
    pub(super) fn psignb_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = op1;
        for n in 0..16usize {
                let sign = (op2.xmm_sbyte(n) > 0) as i32 - (op2.xmm_sbyte(n) < 0) as i32;
                result.set_xmm_sbyte(n, ((op1.xmm_sbyte(n) as i32) * sign) as i8);
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSIGNW VdqWdq (66 0F 38 09) - Negate/Zero/Keep Words Based on Sign (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_psignw> / simd_int.h
    pub(super) fn psignw_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = op1;
        for n in 0..8usize {
                let sign = (op2.xmm16s(n) > 0) as i32 - (op2.xmm16s(n) < 0) as i32;
                result.set_xmm16s(n, ((op1.xmm16s(n) as i32) * sign) as i16);
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// PSIGND VdqWdq (66 0F 38 0A) - Negate/Zero/Keep Dwords Based on Sign (128-bit)
    /// Bochs: HANDLE_SSE_2OP<xmm_psignd> / simd_int.h
    pub(super) fn psignd_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_read_op2_xmm(instr)?;

        let mut result = op1;
        for n in 0..4usize {
                let sign = (op2.xmm32s(n) > 0) as i64 - (op2.xmm32s(n) < 0) as i64;
                result.set_xmm32s(n, ((op1.xmm32s(n) as i64) * sign) as i32);
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
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
                    result.set_xmm64u(0, (result.xmm64u(0) >> bit_shift)
                        | (result.xmm64u(1) << (64 - bit_shift)));
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
                    result.set_xmm64u(0, (op2.xmm64u(0) >> bit_shift)
                        | (op2.xmm64u(1) << (64 - bit_shift)));
                    result.set_xmm64u(1, (op2.xmm64u(1) >> bit_shift)
                        | (op1.xmm64u(0) << (64 - bit_shift)));
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

        for n in 0..16usize {
                if mask.xmm_sbyte(n) < 0 {
                    op1.set_xmmubyte(n, op2.xmmubyte(n));
                }
        }
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

        // Clear OF, SF, AF, ZF, PF, CF
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::AF | EFlags::ZF | EFlags::PF | EFlags::CF);

            if (op2.xmm64u(0) & op1.xmm64u(0)) == 0
                && (op2.xmm64u(1) & op1.xmm64u(1)) == 0
            {
                self.eflags.insert(EFlags::ZF);
            }

            if (op2.xmm64u(0) & !op1.xmm64u(0)) == 0
                && (op2.xmm64u(1) & !op1.xmm64u(1)) == 0
            {
                self.eflags.insert(EFlags::CF);
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

}
