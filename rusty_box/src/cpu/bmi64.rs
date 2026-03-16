//! 64-bit BMI1, BMI2, and ADX instruction handlers.
//! Matching Bochs bmi64.cc.
//!
//! All handlers support both register (mod=11) and memory (mod!=11) operands
//! for the r/m field, matching Bochs _R and _M variants.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =====================================================================
    // Internal helpers
    // =====================================================================

    /// Clear OF, SF, ZF, AF, CF; set SF if sign bit set, set ZF if zero.
    /// Leaves PF unchanged (matching Bochs SET_FLAGS_OSZAxC_LOGIC_64).
    fn set_flags_oszaxc_logic_64(&mut self, result: u64) {
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::ZF | EFlags::AF | EFlags::CF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        }
        if (result & 0x8000_0000_0000_0000) != 0 {
            self.eflags.insert(EFlags::SF);
        }
    }

    /// Read 64-bit r/m operand: register if mod=11, memory read otherwise.
    /// `reg_idx` is the register index from the instruction field (only used when mod=11).
    #[inline]
    fn read_eq64(&mut self, instr: &Instruction, reg_idx: u8) -> super::Result<u64> {
        if instr.mod_c0() {
            Ok(self.get_gpr64(reg_idx as usize))
        } else {
            let seg = BxSegregs::from(instr.seg());
            let offset = self.resolve_addr64(instr);
            self.read_virtual_qword_64(seg, offset)
        }
    }

    // =====================================================================
    // BMI1 — 64-bit
    // =====================================================================

    /// ANDN r64, r64, r/m64 — `~vvv & rm`
    /// Bochs bmi64.cc: ANDN_GqBqEq{R,M}
    /// Our decoder: src1=rm, src2=vvv (Bochs: src1=vvv, src2=rm)
    pub fn andn_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.src2() as usize); // vvv — the one to negate
        let op2 = self.read_eq64(instr, instr.src1())?; // rm — the one to AND with
        let result = !op1 & op2;
        self.set_flags_oszaxc_logic_64(result);
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSI r64, r/m64 — `(-src) & src`, CF = (src != 0)
    /// Bochs bmi64.cc: BLSI_BqEq{R,M}
    /// Group VEX: dst=rm (source), src2=vvv (destination in Bochs)
    pub fn blsi_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_eq64(instr, instr.src())?;
        let tmp_cf = op1 != 0;
        let result = (op1.wrapping_neg()) & op1;
        self.set_flags_oszaxc_logic_64(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        // Group VEX: result goes to vvv (src2), not rm (dst)
        self.set_gpr64(instr.src2() as usize, result);
        Ok(())
    }

    /// BLSMSK r64, r/m64 — `(src - 1) ^ src`, CF = (src == 0)
    /// Bochs bmi64.cc: BLSMSK_BqEq{R,M}
    pub fn blsmsk_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_eq64(instr, instr.src())?;
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) ^ op1;
        self.set_flags_oszaxc_logic_64(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr64(instr.src2() as usize, result);
        Ok(())
    }

    /// BLSR r64, r/m64 — `(src - 1) & src`, CF = (src == 0)
    /// Bochs bmi64.cc: BLSR_BqEq{R,M}
    pub fn blsr_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_eq64(instr, instr.src())?;
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) & op1;
        self.set_flags_oszaxc_logic_64(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr64(instr.src2() as usize, result);
        Ok(())
    }

    /// BEXTR r64, r/m64, r64 — extract bit field
    /// Bochs bmi64.cc: BEXTR_GqEqBq{R,M}
    pub fn bextr_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u16;
        let start = (control & 0xff) as u32;
        let len = (control >> 8) as u32;
        let val = self.read_eq64(instr, instr.src1())?;
        let result = bextrq(val, start, len);
        self.set_flags_oszapc_logic_64(result);
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    // =====================================================================
    // BMI2 — 64-bit
    // =====================================================================

    /// MULX r64, r64, r/m64 — unsigned multiply RDX * rm
    /// lo → vvv (src2), hi → nnn (dst). No flags affected.
    /// Bochs bmi64.cc: MULX_GqBqEq{R,M}
    /// Our decoder: src1=rm (multiplier), src2=vvv (low product dest)
    pub fn mulx_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.rdx() as u128;
        let op2 = self.read_eq64(instr, instr.src1())? as u128; // rm = multiplier
        let product = op1 * op2;
        self.set_gpr64(instr.src2() as usize, product as u64); // vvv = low product
        self.set_gpr64(instr.dst() as usize, (product >> 64) as u64);
        Ok(())
    }

    /// RORX r64, r/m64, imm8 — rotate right by imm8 & 0x3f. No flags.
    /// Bochs bmi64.cc: RORX_GqEqIb{R,M}
    pub fn rorx_gq_eq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_eq64(instr, instr.src())?;
        let count = (instr.ib() as u32) & 0x3f;
        if count != 0 {
            op1 = op1.rotate_right(count);
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// SHRX r64, r/m64, r64 — logical shift right by src2 & 0x3f. No flags.
    /// Bochs bmi64.cc: SHRX_GqEqBq{R,M}
    pub fn shrx_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_eq64(instr, instr.src1())?;
        let count = self.get_gpr32(instr.src2() as usize) & 0x3f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// SARX r64, r/m64, r64 — arithmetic shift right by src2 & 0x3f. No flags.
    /// Bochs bmi64.cc: SARX_GqEqBq{R,M}
    pub fn sarx_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_eq64(instr, instr.src1())? as i64;
        let count = self.get_gpr32(instr.src2() as usize) & 0x3f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr64(instr.dst() as usize, op1 as u64);
        Ok(())
    }

    /// SHLX r64, r/m64, r64 — shift left by src2 & 0x3f. No flags.
    /// Bochs bmi64.cc: SHLX_GqEqBq{R,M}
    pub fn shlx_gq_eq_bq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_eq64(instr, instr.src1())?;
        let count = self.get_gpr32(instr.src2() as usize) & 0x3f;
        if count != 0 {
            op1 <<= count;
        }
        self.set_gpr64(instr.dst() as usize, op1);
        Ok(())
    }

    /// BZHI r64, r/m64, r64 — zero high bits from index.
    /// SET_FLAGS_OSZAxC_LOGIC_64, CF = 1 if control >= 64.
    /// Bochs bmi64.cc: BZHI_GqEqBq{R,M}
    pub fn bzhi_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u8;
        let mut op1 = self.read_eq64(instr, instr.src1())?;
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
    /// Bochs bmi64.cc: PEXT_GqBqEq{R,M}
    /// Our decoder: src1=rm (mask), src2=vvv (source value)
    pub fn pext_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src2() as usize); // vvv = source value
        let mut op2 = self.read_eq64(instr, instr.src1())?; // rm = mask
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
    /// Bochs bmi64.cc: PDEP_GqBqEq{R,M}
    /// Our decoder: src1=rm (mask), src2=vvv (source value)
    pub fn pdep_gq_bq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr64(instr.src2() as usize); // vvv = source value
        let mut op2 = self.read_eq64(instr, instr.src1())?; // rm = mask
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
    /// Bochs bmi64.cc: ADCX_GqEq{R,M}
    pub fn adcx_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_eq64(instr, instr.src())?;
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
    /// Bochs bmi64.cc: ADOX_GqEq{R,M}
    pub fn adox_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr64(instr.dst() as usize);
        let op2 = self.read_eq64(instr, instr.src())?;
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
