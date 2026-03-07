//! 64-bit BMI1, BMI2, and ADX instruction handlers.
//! Matching Bochs bmi64.cc.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =====================================================================
    // Internal: SET_FLAGS_OSZAxC_LOGIC_64 (keep PF unchanged)
    // Bochs bmi64.cc uses this for ANDN, BLSI, BLSMSK, BLSR, BZHI
    // =====================================================================

    /// Clear OF, SF, ZF, CF; set SF if sign bit set, set ZF if zero.
    /// Leaves PF and AF unchanged (matching Bochs SET_FLAGS_OSZAxC_LOGIC_64).
    fn set_flags_oszaxc_logic_64(&mut self, result: u64) {
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::ZF | EFlags::CF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        }
        if (result & 0x8000_0000_0000_0000) != 0 {
            self.eflags.insert(EFlags::SF);
        }
    }

    // =====================================================================
    // BMI1 — 64-bit
    // =====================================================================

    /// ANDN r64, r64, r/m64 — `~src1 & src2`
    /// Bochs bmi64.cc: ANDN_GqBqEqR
    pub fn andn_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.src1() as usize);
        let op2 = self.get_gpr64(instr.src2() as usize);
        let result = !op1 & op2;
        self.set_flags_oszaxc_logic_64(result);
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSI r64, r/m64 — `(-src) & src`, CF = (src != 0)
    /// Bochs bmi64.cc: BLSI_BqEqR
    pub fn blsi_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.src() as usize);
        let tmp_cf = op1 != 0;
        let result = (op1.wrapping_neg()) & op1;
        self.set_flags_oszaxc_logic_64(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSMSK r64, r/m64 — `(src - 1) ^ src`, CF = (src == 0)
    /// Bochs bmi64.cc: BLSMSK_BqEqR
    pub fn blsmsk_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.src() as usize);
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) ^ op1;
        self.set_flags_oszaxc_logic_64(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSR r64, r/m64 — `(src - 1) & src`, CF = (src == 0)
    /// Bochs bmi64.cc: BLSR_BqEqR
    pub fn blsr_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.src() as usize);
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) & op1;
        self.set_flags_oszaxc_logic_64(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// BEXTR r64, r/m64, r64 — extract bit field
    /// Bochs bmi64.cc: BEXTR_GqEqBqR — SET_FLAGS_OSZAPC_LOGIC_64
    pub fn bextr_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u16;
        let start = (control & 0xff) as u32;
        let len = (control >> 8) as u32;
        let val = self.get_gpr64(instr.src1() as usize);
        let result = bextrq(val, start, len);
        self.set_flags_oszapc_logic_64(result);
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    // =====================================================================
    // BMI2 — 64-bit
    // =====================================================================

    /// MULX r64, r64, r/m64 — unsigned multiply RDX * src2
    /// lo → src1 (VEX.vvvv), hi → dst (ModRM.reg). No flags affected.
    /// Bochs bmi64.cc: MULX_GqBqEqR
    pub fn mulx_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.rdx() as u128;
        let op2 = self.get_gpr64(instr.src2() as usize) as u128;
        let product = op1 * op2;
        self.set_gpr64(instr.src1() as usize, product as u64);
        self.set_gpr64(instr.dst() as usize, (product >> 64) as u64);
        Ok(())
    }

    /// RORX r64, r/m64, imm8 — rotate right by imm8 & 0x3f. No flags.
    /// Bochs bmi64.cc: RORX_GqEqIbR
    pub fn rorx_gq_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src() as usize);
        let count = (instr.ib() as u32) & 0x3f;
        if count != 0 {
            op1 = op1.rotate_right(count);
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// SHRX r64, r/m64, r64 — logical shift right by src2 & 0x3f. No flags.
    /// Bochs bmi64.cc: SHRX_GqEqBqR — reads src2 as 32-bit then masks
    pub fn shrx_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src1() as usize);
        let count = self.get_gpr32(instr.src2() as usize) & 0x3f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// SARX r64, r/m64, r64 — arithmetic shift right by src2 & 0x3f. No flags.
    /// Bochs bmi64.cc: SARX_GqEqBqR
    pub fn sarx_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src1() as usize) as i64;
        let count = self.get_gpr32(instr.src2() as usize) & 0x3f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr64(instr.dst() as usize, op1 as u64);
        Ok(())
    }

    /// SHLX r64, r/m64, r64 — shift left by src2 & 0x3f. No flags.
    /// Bochs bmi64.cc: SHLX_GqEqBqR
    pub fn shlx_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src1() as usize);
        let count = self.get_gpr32(instr.src2() as usize) & 0x3f;
        if count != 0 {
            op1 <<= count;
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// BZHI r64, r/m64, r64 — zero high bits from index.
    /// SET_FLAGS_OSZAxC_LOGIC_64, CF = 1 if control >= 64.
    /// Bochs bmi64.cc: BZHI_GqEqBqR
    pub fn bzhi_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        // Bochs: BX_READ_8BIT_REGL(i->src2()) — low byte of register
        let control = self.get_gpr32(instr.src2() as usize) as u8;
        let mut op1 = self.get_gpr64(instr.src1() as usize);
        let tmp_cf;

        if (control as u32) < 64 {
            let mask = (1u64 << control) - 1;
            op1 &= mask;
            tmp_cf = false;
        } else {
            tmp_cf = true;
        }

        self.set_flags_oszaxc_logic_64(op1);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// PEXT r64, r64, r/m64 — parallel bit extract. No flags.
    /// Bochs bmi64.cc: PEXT_GqBqEqR
    pub fn pext_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src1() as usize);
        let mut op2 = self.get_gpr64(instr.src2() as usize);
        let mut result: u64 = 0;
        let mut wr_mask: u64 = 1;

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

        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// PDEP r64, r64, r/m64 — parallel bit deposit. No flags.
    /// Bochs bmi64.cc: PDEP_GqBqEqR
    pub fn pdep_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src1() as usize);
        let mut op2 = self.get_gpr64(instr.src2() as usize);
        let mut result: u64 = 0;
        let mut wr_mask: u64 = 1;

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

        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    // =====================================================================
    // ADX — 64-bit (also in Bochs bmi64.cc)
    // =====================================================================

    /// ADCX r64, r/m64 — add with CF carry-in, update CF only.
    /// Bochs bmi64.cc: ADCX_GqEqR
    pub fn adcx_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let cf_in = if self.eflags.contains(EFlags::CF) {
            1u64
        } else {
            0u64
        };
        let sum = op1.wrapping_add(op2).wrapping_add(cf_in);
        self.set_gpr64(instr.dst() as usize, sum);

        let carry_out = (op1 & op2) | ((op1 | op2) & !sum);
        if (carry_out >> 63) & 1 != 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        Ok(())
    }

    /// ADOX r64, r/m64 — add with OF carry-in, update OF only.
    /// Bochs bmi64.cc: ADOX_GqEqR
    pub fn adox_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.get_gpr64(instr.src() as usize);
        let of_in = if self.eflags.contains(EFlags::OF) {
            1u64
        } else {
            0u64
        };
        let sum = op1.wrapping_add(op2).wrapping_add(of_in);
        self.set_gpr64(instr.dst() as usize, sum);

        let carry_out = (op1 & op2) | ((op1 | op2) & !sum);
        if (carry_out >> 63) & 1 != 0 {
            self.eflags.insert(EFlags::OF);
        } else {
            self.eflags.remove(EFlags::OF);
        }
        Ok(())
    }
}

// =========================================================================
// BEXTR helper (matching Bochs scalar_arith.h bextrq)
// =========================================================================

fn bextrq(val: u64, start: u32, len: u32) -> u64 {
    let start = start & 0xff;
    let len = len & 0xff;
    if start >= 64 {
        return 0;
    }
    let shifted = val >> start;
    if len >= 64 {
        return shifted;
    }
    shifted & ((1u64 << len) - 1)
}
