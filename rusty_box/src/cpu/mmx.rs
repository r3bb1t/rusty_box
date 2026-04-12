//! MMX instruction set implementation
//!
//! Based on Bochs cpu/mmx.cc
//!
//! Implements all MMX instructions including:
//! - Data movement (MOVD, MOVQ, EMMS)
//! - Packed arithmetic (PADD, PSUB, PMULL, PMADDWD, etc.)
//! - Packed logical (PAND, POR, PXOR, PANDN)
//! - Packed shift (PSLL, PSRL, PSRA by register and immediate)
//! - Packed compare (PCMPEQ, PCMPGT)
//! - Pack/Unpack (PUNPCKL/H, PACKS, PACKUS)
//! - SSE-era MMX extensions (PSHUFW, PINSRW, PEXTRW, PMOVMSKB, etc.)
//! - SSSE3 MMX extensions (PSHUFB, PHADD, PHSUB, PSIGN, PABS, PALIGNR)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    i387::BxPackedRegister,
};

// ============================================================================
// Saturation helpers (matching Bochs mmx.cc inline functions)
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

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // MMX infrastructure
    // ========================================================================

    /// Transition from FPU to MMX state.
    /// Bochs: BX_CPU_C::prepareFPU2MMX() from proc_ctrl.cc
    /// Sets TOS=0 and all FPU tags to valid (0).
    #[inline]
    pub(super) fn prepare_fpu2mmx(&mut self) {
        self.the_i387.tos = 0;
        self.the_i387.twd = 0; // all tags = valid
    }

    /// Read MMX register by physical index (NOT TOS-rotated).
    /// MMX registers alias the significand (64-bit) of FPU st_space.
    /// Bochs: BX_READ_MMX_REG(index)
    #[inline]
    pub(super) fn read_mmx_reg(&self, index: u8) -> BxPackedRegister {
        let signif = self.the_i387.st_space[index as usize & 7].signif;
        BxPackedRegister { bytes: (signif).to_le_bytes() }
    }

    /// Write MMX register by physical index.
    /// Also sets sign_exp = 0xFFFF (marking as MMX-modified).
    /// Bochs: BX_WRITE_MMX_REG(index, val)
    #[inline]
    pub(super) fn write_mmx_reg(&mut self, index: u8, val: BxPackedRegister) {
        let reg = &mut self.the_i387.st_space[index as usize & 7];
        reg.signif = val.U64();
        reg.sign_exp = 0xFFFF;
    }

    /// Read the op2 operand: register if modC0, else read qword from memory.
    /// This is the most common pattern in MMX instructions.
    #[inline]
    fn mmx_read_op2_qq(&mut self, instr: &Instruction) -> super::Result<BxPackedRegister> {
        if instr.mod_c0() {
            Ok(self.read_mmx_reg(instr.src1()))
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.v_read_qword(seg, eaddr)?;
            Ok(BxPackedRegister { bytes: (val).to_le_bytes() })
        }
    }

    /// Read op2 as dword (for PUNPCKL* instructions that read 32-bit from memory)
    #[inline]
    fn mmx_read_op2_qd(&mut self, instr: &Instruction) -> super::Result<BxPackedRegister> {
        if instr.mod_c0() {
            Ok(self.read_mmx_reg(instr.src1()))
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.v_read_dword(seg, eaddr)? as u64;
            Ok(BxPackedRegister { bytes: (val).to_le_bytes() })
        }
    }

    // ========================================================================
    // SSSE3 MMX-register forms (0F 38 xx / 0F 3A xx)
    // Bochs 
    // ========================================================================

    /// PSHUFB PqQq (0F 38 00) - Packed Shuffle Bytes
    /// Bochs 
    pub(super) fn pshufb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut result = BxPackedRegister { bytes: [0; 8] };
        for j in 0..8u8 {
            let mask = op2.Ubyte(j as usize);
            if mask & 0x80 != 0 {
                result.set_Ubyte(j as usize, 0);
            } else {
                result.set_Ubyte(j as usize, op1.Ubyte((mask & 7) as usize));
            }
        }
        self.write_mmx_reg(instr.dst(), result);
        Ok(())
    }

    /// PHADDW PqQq (0F 38 01) - Packed Horizontal Add Words
    /// Bochs 
    pub(super) fn phaddw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U16(0, op1.U16(0).wrapping_add(op1.U16(1)));
            r.set_U16(1, op1.U16(2).wrapping_add(op1.U16(3)));
            r.set_U16(2, op2.U16(0).wrapping_add(op2.U16(1)));
            r.set_U16(3, op2.U16(2).wrapping_add(op2.U16(3)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PHADDD PqQq (0F 38 02) - Packed Horizontal Add Dwords
    /// Bochs 
    pub(super) fn phaddd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U32(0, op1.U32(0).wrapping_add(op1.U32(1)));
            r.set_U32(1, op2.U32(0).wrapping_add(op2.U32(1)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PHADDSW PqQq (0F 38 03) - Packed Horizontal Add Saturate Words
    /// Bochs 
    pub(super) fn phaddsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_S16(0, saturate_dword_s_to_word_s(op1.S16(0) as i32 + op1.S16(1) as i32));
            r.set_S16(1, saturate_dword_s_to_word_s(op1.S16(2) as i32 + op1.S16(3) as i32));
            r.set_S16(2, saturate_dword_s_to_word_s(op2.S16(0) as i32 + op2.S16(1) as i32));
            r.set_S16(3, saturate_dword_s_to_word_s(op2.S16(2) as i32 + op2.S16(3) as i32));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMADDUBSW PqQq (0F 38 04) - Multiply Unsigned/Signed Bytes, Add Pairs
    /// Bochs 
    pub(super) fn pmaddubsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                let t = (op1.Ubyte(j * 2) as i32) * (op2.Sbyte(j * 2) as i32)
                    + (op1.Ubyte(j * 2 + 1) as i32) * (op2.Sbyte(j * 2 + 1) as i32);
                r.set_S16(j, saturate_dword_s_to_word_s(t));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PHSUBW PqQq (0F 38 05) - Packed Horizontal Subtract Words
    /// Bochs 
    pub(super) fn phsubw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U16(0, op1.U16(0).wrapping_sub(op1.U16(1)));
            r.set_U16(1, op1.U16(2).wrapping_sub(op1.U16(3)));
            r.set_U16(2, op2.U16(0).wrapping_sub(op2.U16(1)));
            r.set_U16(3, op2.U16(2).wrapping_sub(op2.U16(3)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PHSUBD PqQq (0F 38 06) - Packed Horizontal Subtract Dwords
    /// Bochs 
    pub(super) fn phsubd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U32(0, op1.U32(0).wrapping_sub(op1.U32(1)));
            r.set_U32(1, op2.U32(0).wrapping_sub(op2.U32(1)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PHSUBSW PqQq (0F 38 07) - Packed Horizontal Subtract Saturate Words
    /// Bochs 
    pub(super) fn phsubsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_S16(0, saturate_dword_s_to_word_s(op1.S16(0) as i32 - op1.S16(1) as i32));
            r.set_S16(1, saturate_dword_s_to_word_s(op1.S16(2) as i32 - op1.S16(3) as i32));
            r.set_S16(2, saturate_dword_s_to_word_s(op2.S16(0) as i32 - op2.S16(1) as i32));
            r.set_S16(3, saturate_dword_s_to_word_s(op2.S16(2) as i32 - op2.S16(3) as i32));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSIGNB PqQq (0F 38 08) - Negate/Zero/Keep Bytes Based on Sign
    /// Bochs 
    pub(super) fn psignb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = op1;
            for j in 0..8usize {
                if op2.Sbyte(j) < 0 {
                    r.set_Sbyte(j, -(op1.Sbyte(j) as i16) as i8);
                } else if op2.Sbyte(j) == 0 {
                    r.set_Ubyte(j, 0);
                }
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSIGNW PqQq (0F 38 09)
    /// Bochs 
    pub(super) fn psignw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = op1;
            for j in 0..4usize {
                if op2.S16(j) < 0 {
                    r.set_S16(j, -(op1.S16(j) as i32) as i16);
                } else if op2.S16(j) == 0 {
                    r.set_U16(j, 0);
                }
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSIGND PqQq (0F 38 0A)
    /// Bochs 
    pub(super) fn psignd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = op1;
            for j in 0..2usize {
                if op2.S32(j) < 0 {
                    r.set_S32(j, -(op1.S32(j) as i64) as i32);
                } else if op2.S32(j) == 0 {
                    r.set_U32(j, 0);
                }
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMULHRSW PqQq (0F 38 0B)
    /// Bochs 
    pub(super) fn pmulhrsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                let t = (((op1.S16(j) as i32) * (op2.S16(j) as i32)) >> 14) + 1;
                r.set_S16(j, (t >> 1) as i16);
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PABSB PqQq (0F 38 1C) - Packed Absolute Value Bytes
    /// Bochs 
    pub(super) fn pabsb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Sbyte(j, if op2.Sbyte(j) < 0 {
                    -(op2.Sbyte(j) as i16) as i8
                } else {
                    op2.Sbyte(j)
                });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PABSW PqQq (0F 38 1D)
    /// Bochs 
    pub(super) fn pabsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_S16(j, if op2.S16(j) < 0 {
                    -(op2.S16(j) as i32) as i16
                } else {
                    op2.S16(j)
                });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PABSD PqQq (0F 38 1E)
    /// Bochs 
    pub(super) fn pabsd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..2usize {
                r.set_S32(j, if op2.S32(j) < 0 {
                    -(op2.S32(j) as i64) as i32
                } else {
                    op2.S32(j)
                });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PALIGNR PqQqIb (0F 3A 0F) - Byte-align concatenated qwords
    /// Bochs 
    pub(super) fn palignr_pq_qq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let shift = (instr.ib() as u32) * 8;
        let r = if shift == 0 {
            op2
        } else if shift < 64 {
                BxPackedRegister { bytes: ((op2.U64() >> shift) | (op1.U64() << (64 - shift))).to_le_bytes() }
        } else if shift == 64 {
            op1
        } else if shift < 128 {
                BxPackedRegister { bytes: (op1.U64() >> (shift - 64)).to_le_bytes() }
        } else {
            BxPackedRegister { bytes: [0; 8] }
        };
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    // ========================================================================
    // MMX unpack low (0F 60-62) — Bochs 
    // ========================================================================

    /// PUNPCKLBW PqQd (0F 60) — Unpack Low Bytes
    /// Bochs 
    pub(super) fn punpcklbw_pq_qd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qd(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_Ubyte(7, op2.Ubyte(3));
            r.set_Ubyte(6, op1.Ubyte(3));
            r.set_Ubyte(5, op2.Ubyte(2));
            r.set_Ubyte(4, op1.Ubyte(2));
            r.set_Ubyte(3, op2.Ubyte(1));
            r.set_Ubyte(2, op1.Ubyte(1));
            r.set_Ubyte(1, op2.Ubyte(0));
            r.set_Ubyte(0, op1.Ubyte(0));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PUNPCKLWD PqQd (0F 61) — Unpack Low Words
    /// Bochs 
    pub(super) fn punpcklwd_pq_qd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qd(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U16(3, op2.U16(1));
            r.set_U16(2, op1.U16(1));
            r.set_U16(1, op2.U16(0));
            r.set_U16(0, op1.U16(0));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PUNPCKLDQ PqQd (0F 62) — Unpack Low Dwords
    /// Bochs 
    pub(super) fn punpckldq_pq_qd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qd(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U32(1, op2.U32(0));
            r.set_U32(0, op1.U32(0));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    // ========================================================================
    // Pack and Compare (0F 63-6B) — Bochs 
    // ========================================================================

    /// PACKSSWB PqQq (0F 63) — Pack Signed Words to Signed Bytes
    /// Bochs 
    pub(super) fn packsswb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_Sbyte(0, saturate_word_s_to_byte_s(op1.S16(0)));
            r.set_Sbyte(1, saturate_word_s_to_byte_s(op1.S16(1)));
            r.set_Sbyte(2, saturate_word_s_to_byte_s(op1.S16(2)));
            r.set_Sbyte(3, saturate_word_s_to_byte_s(op1.S16(3)));
            r.set_Sbyte(4, saturate_word_s_to_byte_s(op2.S16(0)));
            r.set_Sbyte(5, saturate_word_s_to_byte_s(op2.S16(1)));
            r.set_Sbyte(6, saturate_word_s_to_byte_s(op2.S16(2)));
            r.set_Sbyte(7, saturate_word_s_to_byte_s(op2.S16(3)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PCMPGTB PqQq (0F 64) — Compare Greater Than Bytes
    /// Bochs 
    pub(super) fn pcmpgtb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, if op1.Sbyte(j) > op2.Sbyte(j) { 0xff } else { 0 });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PCMPGTW PqQq (0F 65)
    /// Bochs 
    pub(super) fn pcmpgtw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, if op1.S16(j) > op2.S16(j) { 0xffff } else { 0 });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PCMPGTD PqQq (0F 66)
    /// Bochs 
    pub(super) fn pcmpgtd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..2usize {
                r.set_U32(j, if op1.S32(j) > op2.S32(j) {
                    0xffffffff
                } else {
                    0
                });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PACKUSWB PqQq (0F 67) — Pack Signed Words to Unsigned Bytes
    /// Bochs 
    pub(super) fn packuswb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_Ubyte(0, saturate_word_s_to_byte_u(op1.S16(0)));
            r.set_Ubyte(1, saturate_word_s_to_byte_u(op1.S16(1)));
            r.set_Ubyte(2, saturate_word_s_to_byte_u(op1.S16(2)));
            r.set_Ubyte(3, saturate_word_s_to_byte_u(op1.S16(3)));
            r.set_Ubyte(4, saturate_word_s_to_byte_u(op2.S16(0)));
            r.set_Ubyte(5, saturate_word_s_to_byte_u(op2.S16(1)));
            r.set_Ubyte(6, saturate_word_s_to_byte_u(op2.S16(2)));
            r.set_Ubyte(7, saturate_word_s_to_byte_u(op2.S16(3)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PUNPCKHBW PqQq (0F 68) — Unpack High Bytes
    /// Bochs 
    pub(super) fn punpckhbw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_Ubyte(0, op1.Ubyte(4));
            r.set_Ubyte(1, op2.Ubyte(4));
            r.set_Ubyte(2, op1.Ubyte(5));
            r.set_Ubyte(3, op2.Ubyte(5));
            r.set_Ubyte(4, op1.Ubyte(6));
            r.set_Ubyte(5, op2.Ubyte(6));
            r.set_Ubyte(6, op1.Ubyte(7));
            r.set_Ubyte(7, op2.Ubyte(7));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PUNPCKHWD PqQq (0F 69) — Unpack High Words
    /// Bochs 
    pub(super) fn punpckhwd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U16(0, op1.U16(2));
            r.set_U16(1, op2.U16(2));
            r.set_U16(2, op1.U16(3));
            r.set_U16(3, op2.U16(3));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PUNPCKHDQ PqQq (0F 6A) — Unpack High Dwords
    /// Bochs 
    pub(super) fn punpckhdq_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U32(0, op1.U32(1));
            r.set_U32(1, op2.U32(1));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PACKSSDW PqQq (0F 6B) — Pack Signed Dwords to Signed Words
    /// Bochs 
    pub(super) fn packssdw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_S16(0, saturate_dword_s_to_word_s(op1.S32(0)));
            r.set_S16(1, saturate_dword_s_to_word_s(op1.S32(1)));
            r.set_S16(2, saturate_dword_s_to_word_s(op2.S32(0)));
            r.set_S16(3, saturate_dword_s_to_word_s(op2.S32(1)));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    // ========================================================================
    // MOVD/MOVQ — Data transfer (0F 6E, 0F 6F, 0F 7E, 0F 7F)
    // Bochs 
    // ========================================================================

    /// MOVD PqEd (0F 6E) — register form: move 32-bit GPR to MMX
    /// Bochs 
    pub(super) fn movd_pq_ed_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let val = self.get_gpr32(instr.src1() as usize) as u64;
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (val).to_le_bytes() });
        Ok(())
    }

    /// MOVD PqEd (0F 6E) — memory form
    /// Bochs 
    pub(super) fn movd_pq_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_dword(seg, eaddr)? as u64;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (val).to_le_bytes() });
        Ok(())
    }

    /// MOVQ PqEq (REX.W + 0F 6E) — register form: move 64-bit GPR to MMX
    /// Bochs 
    pub(super) fn movq_pq_eq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let val = self.get_gpr64(instr.src1() as usize);
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (val).to_le_bytes() });
        Ok(())
    }

    /// MOVQ PqEq (REX.W + 0F 6E) — memory form: load qword from memory to MMX
    /// Bochs 
    pub(super) fn movq_pq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_qword(seg, eaddr)?;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (val).to_le_bytes() });
        Ok(())
    }

    /// MOVQ PqQq (0F 6F) — register form: MMX to MMX
    /// Bochs 
    pub(super) fn movq_pq_qq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let val = self.read_mmx_reg(instr.src1());
        self.write_mmx_reg(instr.dst(), val);
        Ok(())
    }

    /// MOVQ PqQq (0F 6F) — memory form
    /// Bochs 
    pub(super) fn movq_pq_qq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_qword(seg, eaddr)?;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (val).to_le_bytes() });
        Ok(())
    }

    /// PSHUFW PqQqIb (0F 70) — Shuffle Words
    /// Bochs 
    pub(super) fn pshufw_pq_qq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let order = instr.ib();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            r.set_U16(0, op.U16((order & 3) as usize));
            r.set_U16(1, op.U16(((order >> 2) & 3) as usize));
            r.set_U16(2, op.U16(((order >> 4) & 3) as usize));
            r.set_U16(3, op.U16(((order >> 6) & 3) as usize));
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PCMPEQB PqQq (0F 74)
    /// Bochs 
    pub(super) fn pcmpeqb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, if op1.Ubyte(j) == op2.Ubyte(j) {
                    0xff
                } else {
                    0
                });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PCMPEQW PqQq (0F 75)
    /// Bochs 
    pub(super) fn pcmpeqw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, if op1.U16(j) == op2.U16(j) { 0xffff } else { 0 });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PCMPEQD PqQq (0F 76)
    /// Bochs 
    pub(super) fn pcmpeqd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..2usize {
                r.set_U32(j, if op1.U32(j) == op2.U32(j) {
                    0xffffffff
                } else {
                    0
                });
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// EMMS (0F 77) — Empty MMX State
    /// Bochs 
    pub(super) fn emms(&mut self, _instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.the_i387.twd = 0xFFFF; // all tags = empty
        self.the_i387.tos = 0;
        Ok(())
    }

    /// MOVD EdPq (0F 7E) — register form: MMX low dword to 32-bit GPR
    /// Bochs 
    pub(super) fn movd_ed_pq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let op = self.read_mmx_reg(instr.src1());
        let val = op.U32(0);
        self.set_gpr32(instr.dst() as usize, val);
        Ok(())
    }

    /// MOVD EdPq (0F 7E) — memory form
    /// Bochs 
    pub(super) fn movd_ed_pq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op = self.read_mmx_reg(instr.src1());
        let val = op.U32(0);
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        self.v_write_dword(seg, eaddr, val)?;
        self.prepare_fpu2mmx();
        Ok(())
    }

    /// MOVQ EqPq (REX.W + 0F 7E) — register form: store MMX to 64-bit GPR
    /// Bochs 
    pub(super) fn movq_eq_pq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let val = self.read_mmx_reg(instr.src1()).U64();
        self.set_gpr64(instr.dst() as usize, val);
        Ok(())
    }

    /// MOVQ EqPq (REX.W + 0F 7E) — memory form: store MMX qword to memory
    /// Bochs 
    pub(super) fn movq_eq_pq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let val = self.read_mmx_reg(instr.src1()).U64();
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        self.v_write_qword(seg, eaddr, val)?;
        self.prepare_fpu2mmx();
        Ok(())
    }

    /// MOVQ QqPq (0F 7F) / MOVNTQ MqPq (0F E7) — store MMX to memory
    /// Bochs 
    pub(super) fn movq_qq_pq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let val = self.read_mmx_reg(instr.src1()).U64();
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        self.v_write_qword(seg, eaddr, val)?;
        self.prepare_fpu2mmx();
        Ok(())
    }

    // ========================================================================
    // Insert/Extract word (0F C4, 0F C5) — Bochs 
    // ========================================================================

    /// PINSRW PqEwIb (0F C4) — Insert Word
    /// Bochs 
    pub(super) fn pinsrw_pq_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = if instr.mod_c0() {
            self.get_gpr16(instr.src1() as usize)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.v_read_word(seg, eaddr)?
        };
        self.prepare_fpu2mmx();
        op1.set_U16((instr.ib() & 3) as usize, op2);
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PEXTRW GdNqIb (0F C5) — Extract Word
    /// Bochs 
    pub(super) fn pextrw_gd_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let op = self.read_mmx_reg(instr.src1());
        let result = op.U16((instr.ib() & 3) as usize) as u32;
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // Shift right, add qword, multiply (0F D1-D7)
    // ========================================================================

    /// PSRLW PqQq (0F D1)
    /// Bochs 
    pub(super) fn psrlw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let count = op2.U64();
        if count > 15 {
            op1 = BxPackedRegister { bytes: [0; 8] };
        } else {
            let shift = count as u16;
                op1.set_U16(0, op1.U16(0) >> shift);
                op1.set_U16(1, op1.U16(1) >> shift);
                op1.set_U16(2, op1.U16(2) >> shift);
                op1.set_U16(3, op1.U16(3) >> shift);
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PSRLD PqQq (0F D2)
    /// Bochs 
    pub(super) fn psrld_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let count = op2.U64();
        if count > 31 {
            op1 = BxPackedRegister { bytes: [0; 8] };
        } else {
            let shift = count as u32;
                op1.set_U32(0, op1.U32(0) >> shift);
                op1.set_U32(1, op1.U32(1) >> shift);
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PSRLQ PqQq (0F D3)
    /// Bochs 
    pub(super) fn psrlq_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let count = op2.U64();
        if count > 63 {
            op1 = BxPackedRegister { bytes: [0; 8] };
        } else {
                op1.set_U64(op1.U64() >> count);
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PADDQ PqQq (0F D4) — Add Packed Qword
    /// Bochs 
    pub(super) fn paddq_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let r = BxPackedRegister { bytes: (op1.U64().wrapping_add(op2.U64())).to_le_bytes() };
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMULLW PqQq (0F D5) — Multiply Low Words
    /// Bochs 
    pub(super) fn pmullw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, (op1.U16(j) as u32).wrapping_mul(op2.U16(j) as u32) as u16);
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMOVMSKB GdNq (0F D7) — Move Byte Mask
    /// Bochs 
    pub(super) fn pmovmskb_gd_nq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let op = self.read_mmx_reg(instr.src1());
        let mut mask = 0u32;
            for j in 0..8usize {
                if op.Ubyte(j) & 0x80 != 0 {
                    mask |= 1 << j;
                }
            }
        self.set_gpr32(instr.dst() as usize, mask);
        Ok(())
    }

    // ========================================================================
    // Unsigned sub/add with saturation, min/max (0F D8-DE)
    // ========================================================================

    /// PSUBUSB PqQq (0F D8)
    pub(super) fn psubusb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, op1.Ubyte(j).saturating_sub(op2.Ubyte(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSUBUSW PqQq (0F D9)
    pub(super) fn psubusw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, op1.U16(j).saturating_sub(op2.U16(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMINUB PqQq (0F DA)
    pub(super) fn pminub_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, op1.Ubyte(j).min(op2.Ubyte(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PAND PqQq (0F DB)
    pub(super) fn pand_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(
            instr.dst(),
            BxPackedRegister { bytes: (op1.U64() & op2.U64()).to_le_bytes() },
        );
        Ok(())
    }

    /// PADDUSB PqQq (0F DC)
    pub(super) fn paddusb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, op1.Ubyte(j).saturating_add(op2.Ubyte(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PADDUSW PqQq (0F DD)
    pub(super) fn paddusw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, op1.U16(j).saturating_add(op2.U16(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMAXUB PqQq (0F DE)
    pub(super) fn pmaxub_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, op1.Ubyte(j).max(op2.Ubyte(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PANDN PqQq (0F DF) — bitwise AND NOT (~op1 & op2)
    pub(super) fn pandn_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(
            instr.dst(),
            BxPackedRegister { bytes: (!op1.U64() & op2.U64()).to_le_bytes() },
        );
        Ok(())
    }

    // ========================================================================
    // Average, arithmetic shift, multiply high (0F E0-E5)
    // ========================================================================

    /// PAVGB PqQq (0F E0) — Average Bytes
    pub(super) fn pavgb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, ((op1.Ubyte(j) as u16 + op2.Ubyte(j) as u16 + 1) >> 1) as u8);
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSRAW PqQq (0F E1) — Shift Right Arithmetic Words
    pub(super) fn psraw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let count = op2.U64();
        if count == 0 { /* no change */
        } else if count > 15 {
                for j in 0..4usize {
                    op1.set_U16(j, if op1.S16(j) < 0 { 0xffff } else { 0 });
                }
        } else {
                for j in 0..4usize {
                    op1.set_U16(j, (op1.S16(j) >> count as u16) as u16);
                }
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PSRAD PqQq (0F E2) — Shift Right Arithmetic Dwords
    pub(super) fn psrad_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let count = op2.U64();
        if count == 0 { /* no change */
        } else if count > 31 {
                for j in 0..2usize {
                    op1.set_U32(j, if op1.S32(j) < 0 { 0xffffffff } else { 0 });
                }
        } else {
                for j in 0..2usize {
                    op1.set_U32(j, (op1.S32(j) >> count as u32) as u32);
                }
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PAVGW PqQq (0F E3) — Average Words
    pub(super) fn pavgw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, ((op1.U16(j) as u32 + op2.U16(j) as u32 + 1) >> 1) as u16);
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMULHUW PqQq (0F E4) — Multiply High Unsigned Words
    pub(super) fn pmulhuw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, ((op1.U16(j) as u32 * op2.U16(j) as u32) >> 16) as u16);
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMULHW PqQq (0F E5) — Multiply High Signed Words
    pub(super) fn pmulhw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, ((op1.S16(j) as i32 * op2.S16(j) as i32) >> 16) as u16);
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    // ========================================================================
    // Signed sub/add with saturation, min/max (0F E8-EF)
    // ========================================================================

    /// PSUBSB PqQq (0F E8)
    pub(super) fn psubsb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Sbyte(j, saturate_word_s_to_byte_s(op1.Sbyte(j) as i16 - op2.Sbyte(j) as i16));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSUBSW PqQq (0F E9)
    pub(super) fn psubsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_S16(j, saturate_dword_s_to_word_s(op1.S16(j) as i32 - op2.S16(j) as i32));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMINSW PqQq (0F EA)
    pub(super) fn pminsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_S16(j, op1.S16(j).min(op2.S16(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// POR PqQq (0F EB)
    pub(super) fn por_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(
            instr.dst(),
            BxPackedRegister { bytes: (op1.U64() | op2.U64()).to_le_bytes() },
        );
        Ok(())
    }

    /// PADDSB PqQq (0F EC)
    pub(super) fn paddsb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Sbyte(j, saturate_word_s_to_byte_s(op1.Sbyte(j) as i16 + op2.Sbyte(j) as i16));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PADDSW PqQq (0F ED)
    pub(super) fn paddsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_S16(j, saturate_dword_s_to_word_s(op1.S16(j) as i32 + op2.S16(j) as i32));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PMAXSW PqQq (0F EE)
    pub(super) fn pmaxsw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_S16(j, op1.S16(j).max(op2.S16(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PXOR PqQq (0F EF)
    pub(super) fn pxor_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        self.write_mmx_reg(
            instr.dst(),
            BxPackedRegister { bytes: (op1.U64() ^ op2.U64()).to_le_bytes() },
        );
        Ok(())
    }

    // ========================================================================
    // Shift left, multiply-add, SAD, MASKMOVQ (0F F1-F7)
    // ========================================================================

    /// PSLLW PqQq (0F F1)
    pub(super) fn psllw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let count = op2.U64();
        if count > 15 {
            op1 = BxPackedRegister { bytes: [0; 8] };
        } else {
                for j in 0..4usize {
                    op1.set_U16(j, op1.U16(j) << count as u16);
                }
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PSLLD PqQq (0F F2)
    pub(super) fn pslld_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let count = op2.U64();
        if count > 31 {
            op1 = BxPackedRegister { bytes: [0; 8] };
        } else {
                op1.set_U32(0, op1.U32(0) << count as u32);
                op1.set_U32(1, op1.U32(1) << count as u32);
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PSLLQ PqQq (0F F3)
    pub(super) fn psllq_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let mut op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let count = op2.U64();
        if count > 63 {
            op1 = BxPackedRegister { bytes: [0; 8] };
        } else {
                op1.set_U64(op1.U64() << count);
        }
        self.write_mmx_reg(instr.dst(), op1);
        Ok(())
    }

    /// PMULUDQ PqQq (0F F4) — Multiply Unsigned Dwords to Qword
    pub(super) fn pmuludq_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let val = (op1.U32(0) as u64) * (op2.U32(0) as u64);
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (val).to_le_bytes() });
        Ok(())
    }

    /// PMADDWD PqQq (0F F5) — Multiply and Add Packed Words
    /// Bochs  — with 0x80008000 overflow guard
    pub(super) fn pmaddwd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut r = BxPackedRegister { bytes: [0; 8] };
            if op1.U32(0) == 0x80008000 && op2.U32(0) == 0x80008000 {
                r.set_U32(0, 0x80000000);
            } else {
                r.set_S32(0, (op1.S16(0) as i32) * (op2.S16(0) as i32)
                    + (op1.S16(1) as i32) * (op2.S16(1) as i32));
            }
            if op1.U32(1) == 0x80008000 && op2.U32(1) == 0x80008000 {
                r.set_U32(1, 0x80000000);
            } else {
                r.set_S32(1, (op1.S16(2) as i32) * (op2.S16(2) as i32)
                    + (op1.S16(3) as i32) * (op2.S16(3) as i32));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSADBW PqQq (0F F6) — Sum of Absolute Differences
    pub(super) fn psadbw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();

        let mut temp = 0u16;
            for j in 0..8usize {
                temp += (op1.Ubyte(j) as i16 - op2.Ubyte(j) as i16).unsigned_abs();
            }
        self.write_mmx_reg(instr.dst(), BxPackedRegister { bytes: (temp as u64).to_le_bytes() });
        Ok(())
    }

    /// MASKMOVQ PqNq (0F F7) — Masked Store Bytes
    /// Bochs 
    pub(super) fn maskmovq_pq_nq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();

        let op = self.read_mmx_reg(instr.dst());     // nnn = Pq (data source)
        let mask = self.read_mmx_reg(instr.src1()); // rm = Nq (mask)

        // If mask is all zero, nothing to do
        if mask.U64() == 0 {
            return Ok(());
        }

        // Bochs : bx_address rdi = RDI & i->asize_mask();
        let asize_mask: u64 = if instr.as64_l() != 0 {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() == 0 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        };
        let rdi = self.rdi() & asize_mask;
        let seg = BxSegregs::from(instr.seg());

        // Read-modify-write 8 bytes at [seg:rdi]
        let mut tmp = BxPackedRegister { bytes: (self.v_read_qword(seg, rdi)?).to_le_bytes() };
            for j in 0..8usize {
                if mask.Ubyte(j) & 0x80 != 0 {
                    tmp.set_Ubyte(j, op.Ubyte(j));
                }
            }
        self.v_write_qword(seg, rdi, tmp.U64())?;
        Ok(())
    }

    // ========================================================================
    // Packed integer arithmetic (0F F8-FE)
    // ========================================================================

    /// PSUBB PqQq (0F F8)
    pub(super) fn psubb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, op1.Ubyte(j).wrapping_sub(op2.Ubyte(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSUBW PqQq (0F F9)
    pub(super) fn psubw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, op1.U16(j).wrapping_sub(op2.U16(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSUBD PqQq (0F FA)
    pub(super) fn psubd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..2usize {
                r.set_U32(j, op1.U32(j).wrapping_sub(op2.U32(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PSUBQ PqQq (0F FB)
    pub(super) fn psubq_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let r = BxPackedRegister { bytes: (op1.U64().wrapping_sub(op2.U64())).to_le_bytes() };
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PADDB PqQq (0F FC)
    pub(super) fn paddb_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..8usize {
                r.set_Ubyte(j, op1.Ubyte(j).wrapping_add(op2.Ubyte(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PADDW PqQq (0F FD)
    pub(super) fn paddw_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..4usize {
                r.set_U16(j, op1.U16(j).wrapping_add(op2.U16(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    /// PADDD PqQq (0F FE)
    pub(super) fn paddd_pq_qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op1 = self.read_mmx_reg(instr.dst());
        let op2 = self.mmx_read_op2_qq(instr)?;
        self.prepare_fpu2mmx();
        let mut r = BxPackedRegister { bytes: [0; 8] };
            for j in 0..2usize {
                r.set_U32(j, op1.U32(j).wrapping_add(op2.U32(j)));
            }
        self.write_mmx_reg(instr.dst(), r);
        Ok(())
    }

    // ========================================================================
    // Immediate-form shift instructions (0F 71-73 GrpA)
    // Bochs 
    // ========================================================================

    /// PSRLW NqIb (0F 71 /2) — Shift Right Logical Words by Immediate
    pub(super) fn psrlw_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift > 15 {
            op = BxPackedRegister { bytes: [0; 8] };
        } else {
                for j in 0..4usize {
                    op.set_U16(j, op.U16(j) >> shift as u16);
                }
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSRAW NqIb (0F 71 /4) — Shift Right Arithmetic Words by Immediate
    pub(super) fn psraw_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift == 0 { /* no-op */
        } else if shift > 15 {
                for j in 0..4usize {
                    op.set_U16(j, if op.S16(j) < 0 { 0xffff } else { 0 });
                }
        } else {
                for j in 0..4usize {
                    op.set_U16(j, (op.S16(j) >> shift as i16) as u16);
                }
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSLLW NqIb (0F 71 /6) — Shift Left Logical Words by Immediate
    pub(super) fn psllw_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift > 15 {
            op = BxPackedRegister { bytes: [0; 8] };
        } else {
                for j in 0..4usize {
                    op.set_U16(j, op.U16(j) << shift as u16);
                }
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSRLD NqIb (0F 72 /2) — Shift Right Logical Dwords by Immediate
    pub(super) fn psrld_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift > 31 {
            op = BxPackedRegister { bytes: [0; 8] };
        } else {
                op.set_U32(0, op.U32(0) >> shift as u32);
                op.set_U32(1, op.U32(1) >> shift as u32);
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSRAD NqIb (0F 72 /4) — Shift Right Arithmetic Dwords by Immediate
    pub(super) fn psrad_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift == 0 { /* no-op */
        } else if shift > 31 {
                for j in 0..2usize {
                    op.set_U32(j, if op.S32(j) < 0 { 0xffffffff } else { 0 });
                }
        } else {
                for j in 0..2usize {
                    op.set_U32(j, (op.S32(j) >> shift as i32) as u32);
                }
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSLLD NqIb (0F 72 /6) — Shift Left Logical Dwords by Immediate
    pub(super) fn pslld_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift > 31 {
            op = BxPackedRegister { bytes: [0; 8] };
        } else {
                op.set_U32(0, op.U32(0) << shift as u32);
                op.set_U32(1, op.U32(1) << shift as u32);
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSRLQ NqIb (0F 73 /2) — Shift Right Logical Qword by Immediate
    pub(super) fn psrlq_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift > 63 {
            op = BxPackedRegister { bytes: [0; 8] };
        } else {
                op.set_U64(op.U64() >> shift as u64);
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    /// PSLLQ NqIb (0F 73 /6) — Shift Left Logical Qword by Immediate
    pub(super) fn psllq_nq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let mut op = self.read_mmx_reg(instr.dst());
        let shift = instr.ib();
        if shift > 63 {
            op = BxPackedRegister { bytes: [0; 8] };
        } else {
                op.set_U64(op.U64() << shift as u64);
        }
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // MOVQ Qq, Pq — register form (MMX reg to MMX reg)
    // Bochs: MOVQ_QqPq in mmx.cc, modC0 path
    // ========================================================================

    /// MOVQ register-to-register form: dst MMX = src MMX
    pub(super) fn movq_qq_pq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.prepare_fpu2mmx();
        let op = self.read_mmx_reg(instr.src1());
        self.write_mmx_reg(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // MOVNTQ Mq, Pq — non-temporal store (always memory)
    // Bochs: MOVNTQ_MqPq in sse_move.cc
    // ========================================================================

    /// MOVNTQ: non-temporal store of MMX register to memory
    /// Non-temporal hint is ignored in emulation — same as MOVQ store
    /// Bochs : "do not cause FPU2MMX transition if memory write faults"
    pub(super) fn movntq_mq_pq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        let op = self.read_mmx_reg(instr.src1());
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
            self.v_write_qword(seg, eaddr, op.U64())?;
        // prepare_fpu2mmx after write succeeds — if the write faults,
        // FPU state must not be corrupted (matches Bochs )
        self.prepare_fpu2mmx();
        Ok(())
    }
}
