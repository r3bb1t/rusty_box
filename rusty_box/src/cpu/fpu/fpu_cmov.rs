#![allow(dead_code)]
//! FPU conditional move instructions: FCMOV variants
//! Ported from Bochs cpu/fpu/fpu_cmov.cc

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::Instruction;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// FCMOVB ST(0), ST(j) — Move if below (CF=1)
    pub fn fcmovb_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if self.get_cf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVE ST(0), ST(j) — Move if equal (ZF=1)
    pub fn fcmove_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if self.get_zf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVBE ST(0), ST(j) — Move if below or equal (CF=1 or ZF=1)
    pub fn fcmovbe_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if self.get_cf() || self.get_zf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVU ST(0), ST(j) — Move if unordered (PF=1)
    pub fn fcmovu_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if self.get_pf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVNB ST(0), ST(j) — Move if not below (CF=0)
    pub fn fcmovnb_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if !self.get_cf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVNE ST(0), ST(j) — Move if not equal (ZF=0)
    pub fn fcmovne_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if !self.get_zf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVNBE ST(0), ST(j) — Move if not below or equal (CF=0 and ZF=0)
    pub fn fcmovnbe_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if !self.get_cf() && !self.get_zf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }

    /// FCMOVNU ST(0), ST(j) — Move if not unordered (PF=0)
    pub fn fcmovnu_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            if !self.get_pf() {
                self.write_fpu_reg(self.read_fpu_reg(instr.src() as i32), 0);
            }
        }
        Ok(())
    }
}
