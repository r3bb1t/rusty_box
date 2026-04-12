#![allow(dead_code)]
//! FPU miscellaneous instructions: FXCH, FCHS, FABS, FDECSTP, FINCSTP, FFREE, FFREEP
//! Ported from Bochs cpu/fpu/fpu_misc.cc

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::Instruction;
use super::super::i387::{FPU_EX_STACK_UNDERFLOW, FPU_TAG_EMPTY};
use super::super::softfloat3e::softfloat::{floatx80_abs, floatx80_chs};
use super::super::softfloat3e::specialize::FLOATX80_DEFAULT_NAN;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// FXCH ST(i) — Exchange ST(0) and ST(i)
    pub fn fxch_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        let st0_tag = self.the_i387.fpu_gettagi(0);
        let sti_tag = self.the_i387.fpu_gettagi(instr.src() as i32);

        let mut st0_reg = self.read_fpu_reg(0);
        let mut sti_reg = self.read_fpu_reg(instr.src() as i32);

        self.clear_c1();

        if st0_tag == FPU_TAG_EMPTY as i32 || sti_tag == FPU_TAG_EMPTY as i32 {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if self.the_i387.is_ia_masked() {
                if st0_tag == FPU_TAG_EMPTY as i32 {
                    st0_reg = FLOATX80_DEFAULT_NAN;
                }
                if sti_tag == FPU_TAG_EMPTY as i32 {
                    sti_reg = FLOATX80_DEFAULT_NAN;
                }
            } else {
                return Ok(());
            }
        }

        self.write_fpu_reg(st0_reg, instr.src() as i32);
        self.write_fpu_reg(sti_reg, 0);

        Ok(())
    }

    /// FCHS — Change sign of ST(0)
    pub fn fchs(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            self.clear_c1();
            let st0_reg = self.read_fpu_reg(0);
            self.write_fpu_reg(floatx80_chs(st0_reg), 0);
        }

        Ok(())
    }

    /// FABS — Absolute value of ST(0)
    pub fn fabs_(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
        } else {
            self.clear_c1();
            let st0_reg = self.read_fpu_reg(0);
            self.write_fpu_reg(floatx80_abs(st0_reg), 0);
        }

        Ok(())
    }

    /// FDECSTP — Decrement stack pointer
    pub fn fdecstp(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387.tos = (self.the_i387.tos.wrapping_sub(1)) & 7;

        Ok(())
    }

    /// FINCSTP — Increment stack pointer
    pub fn fincstp(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387.tos = (self.the_i387.tos.wrapping_add(1)) & 7;

        Ok(())
    }

    /// FFREE ST(i) — Set tag to Empty
    pub fn ffree_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387
            .fpu_settagi(FPU_TAG_EMPTY as i32, instr.dst() as i32);

        Ok(())
    }

    /// FFREEP ST(i) — Free and pop (undocumented but used)
    pub fn ffreep_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387
            .fpu_settagi(FPU_TAG_EMPTY as i32, instr.dst() as i32);
        self.the_i387.fpu_pop();

        Ok(())
    }
}
