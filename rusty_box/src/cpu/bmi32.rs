//! 32-bit BMI1, BMI2, and ADX instruction handlers.
//! Matching Bochs bmi32.cc.
//!
//! All handlers support both register (mod=11) and memory (mod!=11) operands
//! for the r/m field, matching Bochs _R and _M variants.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =====================================================================
    // Internal helpers
    // =====================================================================

    /// Clear OF, SF, ZF, AF, CF; set SF if sign bit set, set ZF if zero.
    /// Leaves PF unchanged (matching Bochs SET_FLAGS_OSZAxC_LOGIC_32).
    fn set_flags_oszaxc_logic_32(&mut self, result: u32) {
        self.eflags
            .remove(EFlags::OF | EFlags::SF | EFlags::ZF | EFlags::AF | EFlags::CF);
        if result == 0 {
            self.eflags.insert(EFlags::ZF);
        }
        if (result & 0x8000_0000) != 0 {
            self.eflags.insert(EFlags::SF);
        }
    }

    /// Read 32-bit r/m operand: register if mod=11, memory read otherwise.
    #[inline]
    fn read_ed32(&mut self, instr: &Instruction, reg_idx: u8) -> super::Result<u32> {
        if instr.mod_c0() {
            Ok(self.get_gpr32(reg_idx as usize))
        } else if self.long64_mode() {
            let seg = BxSegregs::from(instr.seg());
            let offset = self.resolve_addr64(instr);
            self.read_virtual_dword_64(seg, offset)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let offset = self.resolve_addr32(instr);
            self.read_virtual_dword(seg, offset)
        }
    }

    // =====================================================================
    // BMI1 — 32-bit
    // =====================================================================

    /// ANDN r32, r32, r/m32 — `~vvv & rm`
    /// Bochs bmi32.cc: ANDN_GdBdEd{R,M}
    /// Our decoder: src1=rm, src2=vvv
    pub fn andn_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.src2() as usize); // vvv
        let op2 = self.read_ed32(instr, instr.src1())?; // rm
        let result = !op1 & op2;
        self.set_flags_oszaxc_logic_32(result);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    /// BLSI r32, r/m32 — `(-src) & src`, CF = (src != 0)
    /// Bochs bmi32.cc: BLSI_BdEd{R,M}
    /// Group VEX: dst=rm (source), src2=vvv (destination in Bochs)
    pub fn blsi_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_ed32(instr, instr.src())?;
        let tmp_cf = op1 != 0;
        let result = (op1.wrapping_neg()) & op1;
        self.set_flags_oszaxc_logic_32(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.src2() as usize, result);
        Ok(())
    }

    /// BLSMSK r32, r/m32 — `(src - 1) ^ src`, CF = (src == 0)
    /// Bochs bmi32.cc: BLSMSK_BdEd{R,M}
    pub fn blsmsk_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_ed32(instr, instr.src())?;
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) ^ op1;
        self.set_flags_oszaxc_logic_32(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.src2() as usize, result);
        Ok(())
    }

    /// BLSR r32, r/m32 — `(src - 1) & src`, CF = (src == 0)
    /// Bochs bmi32.cc: BLSR_BdEd{R,M}
    pub fn blsr_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_ed32(instr, instr.src())?;
        let tmp_cf = op1 == 0;
        let result = op1.wrapping_sub(1) & op1;
        self.set_flags_oszaxc_logic_32(result);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.src2() as usize, result);
        Ok(())
    }

    /// BEXTR r32, r/m32, r32 — extract bit field
    /// Bochs bmi32.cc: BEXTR_GdEdBd{R,M}
    pub fn bextr_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u16;
        let start = (control & 0xff) as u32;
        let len = (control >> 8) as u32;
        let val = self.read_ed32(instr, instr.src1())?;
        let result = bextrd(val, start, len);
        self.set_flags_oszapc_logic_32(result);
        self.set_gpr32(instr.dst() as usize, result);
        Ok(())
    }

    // =====================================================================
    // BMI2 — 32-bit
    // =====================================================================

    /// MULX r32, r32, r/m32 — unsigned multiply EDX * rm
    /// Bochs bmi32.cc: MULX_GdBdEd{R,M}
    /// Our decoder: src1=rm (multiplier), src2=vvv (low product dest)
    pub fn mulx_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.edx() as u64;
        let op2 = self.read_ed32(instr, instr.src1())? as u64; // rm = multiplier
        let product = op1 * op2;
        self.set_gpr32(instr.src2() as usize, product as u32); // vvv = low product
        self.set_gpr32(instr.dst() as usize, (product >> 32) as u32);
        Ok(())
    }

    /// RORX r32, r/m32, imm8 — rotate right by imm8 & 0x1f. No flags.
    /// Bochs bmi32.cc: RORX_GdEdIb{R,M}
    pub fn rorx_gd_ed_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_ed32(instr, instr.src())?;
        let count = (instr.ib() as u32) & 0x1f;
        if count != 0 {
            op1 = op1.rotate_right(count);
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// SHRX r32, r/m32, r32 — logical shift right by src2 & 0x1f. No flags.
    /// Bochs bmi32.cc: SHRX_GdEdBd{R,M}
    pub fn shrx_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_ed32(instr, instr.src1())?;
        let count = self.get_gpr32(instr.src2() as usize) & 0x1f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// SARX r32, r/m32, r32 — arithmetic shift right by src2 & 0x1f. No flags.
    /// Bochs bmi32.cc: SARX_GdEdBd{R,M}
    pub fn sarx_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_ed32(instr, instr.src1())? as i32;
        let count = self.get_gpr32(instr.src2() as usize) & 0x1f;
        if count != 0 {
            op1 >>= count;
        }
        self.set_gpr32(instr.dst() as usize, op1 as u32);
        Ok(())
    }

    /// SHLX r32, r/m32, r32 — shift left by src2 & 0x1f. No flags.
    /// Bochs bmi32.cc: SHLX_GdEdBd{R,M}
    pub fn shlx_gd_ed_bd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.read_ed32(instr, instr.src1())?;
        let count = self.get_gpr32(instr.src2() as usize) & 0x1f;
        if count != 0 {
            op1 <<= count;
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// BZHI r32, r/m32, r32 — zero high bits from index.
    /// Bochs bmi32.cc: BZHI_GdEdBd{R,M}
    pub fn bzhi_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = self.get_gpr32(instr.src2() as usize) as u8;
        let mut op1 = self.read_ed32(instr, instr.src1())?;
        let tmp_cf = if (control as u32) < 32 {
            let mask = (1u32 << control) - 1;
            op1 &= mask;
            false
        } else {
            true
        };

        self.set_flags_oszaxc_logic_32(op1);
        if tmp_cf {
            self.eflags.insert(EFlags::CF);
        }
        self.set_gpr32(instr.dst() as usize, op1);
        Ok(())
    }

    /// PEXT r32, r32, r/m32 — parallel bit extract. No flags.
    /// Bochs bmi32.cc: PEXT_GdBdEd{R,M}
    /// Our decoder: src1=rm (mask), src2=vvv (source value)
    pub fn pext_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src2() as usize); // vvv = source
        let mut op2 = self.read_ed32(instr, instr.src1())?; // rm = mask
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
    /// Bochs bmi32.cc: PDEP_GdBdEd{R,M}
    /// Our decoder: src1=rm (mask), src2=vvv (source value)
    pub fn pdep_gd_bd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op1 = self.get_gpr32(instr.src2() as usize); // vvv = source
        let mut op2 = self.read_ed32(instr, instr.src1())?; // rm = mask
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
    /// Bochs bmi32.cc: ADCX_GdEd{R,M}
    pub fn adcx_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.dst() as usize);
        let op2 = self.read_ed32(instr, instr.src())?;
        let cf_in = if self.eflags.contains(EFlags::CF) {
            1u32
        } else {
            0u32
        };
        let sum = op1.wrapping_add(op2).wrapping_add(cf_in);
        self.set_gpr32(instr.dst() as usize, sum);

        let carry_out = (op1 & op2) | ((op1 | op2) & !sum);
        if (carry_out >> 31) & 1 != 0 {
            self.eflags.insert(EFlags::CF);
        } else {
            self.eflags.remove(EFlags::CF);
        }
        Ok(())
    }

    /// ADOX r32, r/m32 — add with OF carry-in, update OF only.
    /// Bochs bmi32.cc: ADOX_GdEd{R,M}
    pub fn adox_gd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.get_gpr32(instr.dst() as usize);
        let op2 = self.read_ed32(instr, instr.src())?;
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
