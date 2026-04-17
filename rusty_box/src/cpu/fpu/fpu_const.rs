#![allow(dead_code)]
//! FPU constant-loading instructions: FLD1, FLDZ, FLDL2T, FLDL2E, FLDPI, FLDLG2, FLDLN2
//! Ported from Bochs cpu/fpu/fpu_const.cc

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::Instruction;
use super::super::i387::{FPU_CW_RC, FPU_RC_DOWN, FPU_RC_UP};
use super::super::softfloat3e::softfloat_types::floatx80;

// Exact 80-bit constants from Bochs fpu_const.cc
const CONST_Z: floatx80 = floatx80 {
    signif: 0x0000000000000000,
    sign_exp: 0x0000,
};
const CONST_1: floatx80 = floatx80 {
    signif: 0x8000000000000000,
    sign_exp: 0x3FFF,
};
const CONST_L2T: floatx80 = floatx80 {
    signif: 0xD49A784BCD1B8AFE,
    sign_exp: 0x4000,
};
const CONST_L2E: floatx80 = floatx80 {
    signif: 0xB8AA3B295C17F0BC,
    sign_exp: 0x3FFF,
};
const CONST_PI: floatx80 = floatx80 {
    signif: 0xC90FDAA22168C235,
    sign_exp: 0x4000,
};
const CONST_LG2: floatx80 = floatx80 {
    signif: 0x9A209A84FBCFF799,
    sign_exp: 0x3FFD,
};
const CONST_LN2: floatx80 = floatx80 {
    signif: 0xB17217F7D1CF79AC,
    sign_exp: 0x3FFE,
};

/// Adjust constant for rounding: add `adj` to significand.
/// Used for transcendental constants that are not exactly representable.
#[inline]
fn fpu_round_const(a: floatx80, adj: i64) -> floatx80 {
    floatx80 {
        signif: (a.signif as i64).wrapping_add(adj) as u64,
        sign_exp: a.sign_exp,
    }
}

/// Check if rounding mode is DOWN or CHOP (truncate toward zero).
#[inline]
fn down_or_chop(cwd: u16) -> bool {
    (cwd & FPU_CW_RC & FPU_RC_DOWN) != 0
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// FLDL2T — Load log2(10)
    pub fn fldl2t(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let adj = if (self.the_i387.get_control_word() & FPU_CW_RC) == FPU_RC_UP {
                1
            } else {
                0
            };
            self.the_i387.fpu_push();
            self.write_fpu_reg(fpu_round_const(CONST_L2T, adj), 0);
        }
        Ok(())
    }

    /// FLDL2E — Load log2(e)
    pub fn fldl2e(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let adj = if down_or_chop(self.the_i387.get_control_word()) {
                -1
            } else {
                0
            };
            self.the_i387.fpu_push();
            self.write_fpu_reg(fpu_round_const(CONST_L2E, adj), 0);
        }
        Ok(())
    }

    /// FLDPI — Load pi
    pub fn fldpi(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let adj = if down_or_chop(self.the_i387.get_control_word()) {
                -1
            } else {
                0
            };
            self.the_i387.fpu_push();
            self.write_fpu_reg(fpu_round_const(CONST_PI, adj), 0);
        }
        Ok(())
    }

    /// FLDLG2 — Load log10(2)
    pub fn fldlg2(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let adj = if down_or_chop(self.the_i387.get_control_word()) {
                -1
            } else {
                0
            };
            self.the_i387.fpu_push();
            self.write_fpu_reg(fpu_round_const(CONST_LG2, adj), 0);
        }
        Ok(())
    }

    /// FLDLN2 — Load ln(2)
    pub fn fldln2(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let adj = if down_or_chop(self.the_i387.get_control_word()) {
                -1
            } else {
                0
            };
            self.the_i387.fpu_push();
            self.write_fpu_reg(fpu_round_const(CONST_LN2, adj), 0);
        }
        Ok(())
    }

    /// FLD1 — Load +1.0
    pub fn fld1(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            self.the_i387.fpu_push();
            self.write_fpu_reg(CONST_1, 0);
        }
        Ok(())
    }

    /// FLDZ — Load +0.0
    pub fn fldz(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            self.the_i387.fpu_push();
            self.write_fpu_reg(CONST_Z, 0);
        }
        Ok(())
    }
}
