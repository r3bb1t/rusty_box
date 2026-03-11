//! 32-bit BMI1, BMI2, and ADX instruction handlers.
//! Matching Bochs bmi32.cc.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =====================================================================
    // Internal: SET_FLAGS_OSZAxC_LOGIC_32 (keep PF unchanged)
    // Bochs bmi32.cc uses this for ANDN, BLSI, BLSMSK, BLSR, BZHI
    // =====================================================================

    /// Clear OF, SF, ZF, AF, CF; set SF if sign bit set, set ZF if zero.
    /// Leaves PF unchanged (matching Bochs SET_FLAGS_OSZAxC_LOGIC_32).
    fn set_flags_oszaxc_logic_32(&mut self, result: u32) {
        // Bochs SET_FLAGS_OSZAxC_LOGIC_32: save PF, call SET_FLAGS_OSZAPC_SIZE
        // with carries=0 (clears OF, CF, AF; sets SF, ZF from result), restore PF.
        // Net effect: clears OF, SF, ZF, AF, CF; sets SF/ZF from result; PF unchanged.
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::ZF | EFlags::AF | EFlags::CF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        }
        if (result & 0x8000_0000) != 0 {
            self.eflags.insert(EFlags::SF);
        }
    }

    // =====================================================================
    // BMI1 — 32-bit
    // =====================================================================

    /// ANDN r32, r32, r/m32 — `~src1 & src2`
    /// Bochs bmi32.cc: ANDN_GdBdEdR — SET_FLAGS_OSZAxC_LOGIC_32, PF unchanged
    pub fn andn_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.src1() as usize);
        let op2 = self.get_gpr32(instr.src2() as usize);
        let result = !op1 & op2;
        self.set_flags_oszaxc_logic_32(result);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSI r32, r/m32 — `(-src) & src`, CF = (src != 0)
    /// Bochs bmi32.cc: BLSI_BdEdR
    pub fn blsi_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.src() as usize);
        let tmp_cf = op1 != 0;
        let result = (op1.wrapping_neg()) & op1;
        self.set_flags_oszaxc_logic_32(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSMSK r32, r/m32 — `(src - 1) ^ src`, CF = (src == 0)
    /// Bochs bmi32.cc: BLSMSK_BdEdR
    pub fn blsmsk_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.src() as usize);
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) ^ op1;
        self.set_flags_oszaxc_logic_32(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSR r32, r/m32 — `(src - 1) & src`, CF = (src == 0)
    /// Bochs bmi32.cc: BLSR_BdEdR
    pub fn blsr_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.src() as usize);
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) & op1;
        self.set_flags_oszaxc_logic_32(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// BEXTR r32, r/m32, r32 — extract bit field
    /// Bochs bmi32.cc: BEXTR_GdEdBdR — SET_FLAGS_OSZAPC_LOGIC_32
    pub fn bextr_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u16;
        let start = (control & 0xff) as u32;
        let len = (control >> 8) as u32;
        let val = self.get_gpr32(instr.src1() as usize);
        let result = bextrd(val, start, len);
        self.set_flags_oszapc_logic_32(result);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    // =====================================================================
    // BMI2 — 32-bit
    // =====================================================================

    /// MULX r32, r32, r/m32 — unsigned multiply EDX * src2
    /// lo → src1 (VEX.vvvv), hi → dst (ModRM.reg). No flags affected.
    /// Bochs bmi32.cc: MULX_GdBdEdR
    pub fn mulx_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.edx() as u64;
        let op2 = self.get_gpr32(instr.src2() as usize) as u64;
        let product = op1 * op2;
        self.set_gpr32(instr.src1() as usize, product as u32);
        self.set_gpr32(instr.dst() as usize, (product >> 32) as u32);
        Ok(())
    }

    /// RORX r32, r/m32, imm8 — rotate right by imm8 & 0x1f. No flags.
    /// Bochs bmi32.cc: RORX_GdEdIbR
    pub fn rorx_gd_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src() as usize);
        let count = (instr.ib() as u32) & 0x1f;
        if count != 0 {
            op1 = op1.rotate_right(count);
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// SHRX r32, r/m32, r32 — logical shift right by src2 & 0x1f. No flags.
    /// Bochs bmi32.cc: SHRX_GdEdBdR
    pub fn shrx_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src1() as usize);
        let count = self.get_gpr32(instr.src2() as usize) & 0x1f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// SARX r32, r/m32, r32 — arithmetic shift right by src2 & 0x1f. No flags.
    /// Bochs bmi32.cc: SARX_GdEdBdR
    pub fn sarx_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src1() as usize) as i32;
        let count = self.get_gpr32(instr.src2() as usize) & 0x1f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr32(instr.dst() as usize, op1 as u32);
        Ok(())
    }

    /// SHLX r32, r/m32, r32 — shift left by src2 & 0x1f. No flags.
    /// Bochs bmi32.cc: SHLX_GdEdBdR
    pub fn shlx_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src1() as usize);
        let count = self.get_gpr32(instr.src2() as usize) & 0x1f;
        if count != 0 {
            op1 <<= count;
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// BZHI r32, r/m32, r32 — zero high bits from index.
    /// SET_FLAGS_OSZAxC_LOGIC_32, CF = 1 if control >= 32.
    /// Bochs bmi32.cc: BZHI_GdEdBdR
    pub fn bzhi_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u8;
        let mut op1 = self.get_gpr32(instr.src1() as usize);
        let tmp_cf;

        if (control as u32) < 32 {
            let mask = (1u32 << control) - 1;
            op1 &= mask;
            tmp_cf = false;
        } else {
            tmp_cf = true;
        }

        self.set_flags_oszaxc_logic_32(op1);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// PEXT r32, r32, r/m32 — parallel bit extract. No flags.
    /// Bochs bmi32.cc: PEXT_GdBdEdR
    pub fn pext_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src1() as usize);
        let mut op2 = self.get_gpr32(instr.src2() as usize);
        let mut result: u32 = 0;
        let mut wr_mask: u32 = 1;

        while op2 != 0 {
            if op2 & 1 != 0 {
                if op1 & 1 != 0 {
                    result |= wr_mask;
                }
                wr_mask <<= 1;
            }
            op1 >>= 1;
            op2 >>= 1;
        }

        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// PDEP r32, r32, r/m32 — parallel bit deposit. No flags.
    /// Bochs bmi32.cc: PDEP_GdBdEdR
    pub fn pdep_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src1() as usize);
        let mut op2 = self.get_gpr32(instr.src2() as usize);
        let mut result: u32 = 0;
        let mut wr_mask: u32 = 1;

        while op2 != 0 {
            if op2 & 1 != 0 {
                if op1 & 1 != 0 {
                    result |= wr_mask;
                }
                op1 >>= 1;
            }
            wr_mask <<= 1;
            op2 >>= 1;
        }

        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    // =====================================================================
    // ADX — 32-bit (also in Bochs bmi32.cc)
    // =====================================================================

    /// ADCX r32, r/m32 — add with CF carry-in, update CF only.
    /// Bochs bmi32.cc: ADCX_GdEdR
    pub fn adcx_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.dst() as usize);
        let op2 = self.get_gpr32(instr.src() as usize);
        let cf_in = if self.eflags.contains(EFlags::CF) {
            1u32
        } else {
            0u32
        };
        let sum = op1.wrapping_add(op2).wrapping_add(cf_in);
        self.set_gpr32(instr.dst() as usize, sum);

        // ADD_COUT_VEC: carry-out at each bit position
        let carry_out = (op1 & op2) | ((op1 | op2) & !sum);
        if (carry_out >> 31) & 1 != 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        Ok(())
    }

    /// ADOX r32, r/m32 — add with OF carry-in, update OF only.
    /// Bochs bmi32.cc: ADOX_GdEdR
    pub fn adox_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.dst() as usize);
        let op2 = self.get_gpr32(instr.src() as usize);
        let of_in = if self.eflags.contains(EFlags::OF) {
            1u32
        } else {
            0u32
        };
        let sum = op1.wrapping_add(op2).wrapping_add(of_in);
        self.set_gpr32(instr.dst() as usize, sum);

        let carry_out = (op1 & op2) | ((op1 | op2) & !sum);
        if (carry_out >> 31) & 1 != 0 {
            self.eflags.insert(EFlags::OF);
        } else {
            self.eflags.remove(EFlags::OF);
        }
        Ok(())
    }
}

// =========================================================================
// BEXTR helper (matching Bochs scalar_arith.h bextrd)
// =========================================================================

fn bextrd(val: u32, start: u32, len: u32) -> u32 {
    let start = start & 0xff;
    let len = len & 0xff;
    if start >= 32 {
        return 0;
    }
    let shifted = val >> start;
    if len >= 32 {
        return shifted;
    }
    shifted & ((1u32 << len) - 1)
}
