#![allow(dead_code)]
//! FPU arithmetic instructions: FADD, FMUL, FSUB, FSUBR, FDIV, FDIVR, FSQRT, FRNDINT
//! Ported from Bochs cpu/fpu/fpu_arith.cc

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::{Instruction, BxSegregs};
use super::super::softfloat3e::softfloat::SoftFloatStatus;
use super::super::softfloat3e::extf80_addsub::{extf80_add, extf80_sub};
use super::super::softfloat3e::extf80_mul::extf80_mul;
use super::super::softfloat3e::extf80_div::extf80_div;
use super::super::softfloat3e::extf80_sqrt::extf80_sqrt;
use super::super::softfloat3e::extf80_roundToInt::extf80_round_to_int;
use super::super::softfloat3e::f32_to_extf80::f32_to_extf80;
use super::super::softfloat3e::f64_to_extf80::f64_to_extf80;
use super::super::softfloat3e::i32_to_extf80::i32_to_extf80;
use super::ferr::i387cw_to_softfloat_status_word;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ================================================================
    // FADD variants
    // ================================================================

    /// FADD ST(0), ST(j) -- ST(0) = ST(0) + ST(j)
    pub fn fadd_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(instr.src() as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_add(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FADD ST(i), ST(0) -- ST(i) = ST(i) + ST(0), pop if instr.b1() & 2
    pub fn fadd_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = (instr.b1() & 2) != 0;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.dst() as i32) {
            self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            return Ok(());
        }

        let a = self.read_fpu_reg(instr.dst() as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_add(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FADDP ST(i), ST(0) -- same dispatch as fadd_sti_st0 (pop controlled by b1() & 2)
    pub fn faddp_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fadd_sti_st0(instr)
    }

    /// FADD single real (f32 from memory)
    pub fn fadd_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f32_to_extf80(load_reg, &mut status);
        let result = extf80_add(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FADD double real (f64 from memory)
    pub fn fadd_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f64_to_extf80(load_reg, &mut status);
        let result = extf80_add(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIADD word integer (i16 from memory)
    pub fn fiadd_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_add(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIADD dword integer (i32 from memory)
    pub fn fiadd_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_add(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FMUL variants
    // ================================================================

    /// FMUL ST(0), ST(j) -- ST(0) = ST(0) * ST(j)
    pub fn fmul_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(instr.src() as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_mul(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FMUL ST(i), ST(0) -- ST(i) = ST(i) * ST(0), pop if instr.b1() & 2
    pub fn fmul_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = (instr.b1() & 2) != 0;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.dst() as i32) {
            self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            return Ok(());
        }

        let a = self.read_fpu_reg(instr.dst() as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_mul(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FMULP ST(i), ST(0) -- same dispatch as fmul_sti_st0 (pop controlled by b1() & 2)
    pub fn fmulp_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fmul_sti_st0(instr)
    }

    /// FMUL single real (f32 from memory)
    pub fn fmul_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f32_to_extf80(load_reg, &mut status);
        let result = extf80_mul(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FMUL double real (f64 from memory)
    pub fn fmul_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f64_to_extf80(load_reg, &mut status);
        let result = extf80_mul(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIMUL word integer (i16 from memory)
    pub fn fimul_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_mul(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIMUL dword integer (i32 from memory)
    pub fn fimul_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_mul(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FSUB variants
    // ================================================================

    /// FSUB ST(0), ST(j) -- ST(0) = ST(0) - ST(j)
    pub fn fsub_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(instr.src() as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUBR ST(0), ST(j) -- ST(0) = ST(j) - ST(0) (reverse)
    pub fn fsubr_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        // Reverse: a = ST(j), b = ST(0)
        let a = self.read_fpu_reg(instr.src() as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUB ST(i), ST(0) -- ST(i) = ST(i) - ST(0), pop if instr.b1() & 2
    pub fn fsub_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = (instr.b1() & 2) != 0;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.dst() as i32) {
            self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            return Ok(());
        }

        let a = self.read_fpu_reg(instr.dst() as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FSUBP ST(i), ST(0) -- same dispatch as fsub_sti_st0 (pop controlled by b1() & 2)
    pub fn fsubp_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fsub_sti_st0(instr)
    }

    /// FSUBR ST(i), ST(0) -- ST(i) = ST(0) - ST(i) (reverse), pop if instr.b1() & 2
    pub fn fsubr_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = (instr.b1() & 2) != 0;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.dst() as i32) {
            self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            return Ok(());
        }

        // Reverse: a = ST(0), b = ST(i)
        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(instr.dst() as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FSUBRP ST(i), ST(0) -- same dispatch as fsubr_sti_st0 (pop controlled by b1() & 2)
    pub fn fsubrp_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fsubr_sti_st0(instr)
    }

    /// FSUB single real (f32 from memory) -- ST(0) = ST(0) - f32
    pub fn fsub_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f32_to_extf80(load_reg, &mut status);
        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUBR single real (f32 from memory) -- ST(0) = f32 - ST(0)
    pub fn fsubr_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f32, b = ST(0)
        let b = self.read_fpu_reg(0);
        let a = f32_to_extf80(load_reg, &mut status);
        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUB double real (f64 from memory) -- ST(0) = ST(0) - f64
    pub fn fsub_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f64_to_extf80(load_reg, &mut status);
        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUBR double real (f64 from memory) -- ST(0) = f64 - ST(0)
    pub fn fsubr_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f64, b = ST(0)
        let b = self.read_fpu_reg(0);
        let a = f64_to_extf80(load_reg, &mut status);
        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FISUB word integer (i16 from memory) -- ST(0) = ST(0) - i16
    pub fn fisub_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FISUBR word integer (i16 from memory) -- ST(0) = i16 - ST(0)
    pub fn fisubr_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        // Reverse: a = i16, b = ST(0)
        let a = i32_to_extf80(load_reg as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FISUB dword integer (i32 from memory) -- ST(0) = ST(0) - i32
    pub fn fisub_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(self.read_fpu_reg(0), i32_to_extf80(load_reg), &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FISUBR dword integer (i32 from memory) -- ST(0) = i32 - ST(0)
    pub fn fisubr_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        // Reverse: a = i32, b = ST(0)
        let a = i32_to_extf80(load_reg);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sub(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FDIV variants
    // ================================================================

    /// FDIV ST(0), ST(j) -- ST(0) = ST(0) / ST(j)
    pub fn fdiv_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(instr.src() as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIVR ST(0), ST(j) -- ST(0) = ST(j) / ST(0) (reverse)
    pub fn fdivr_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        // Reverse: a = ST(j), b = ST(0)
        let a = self.read_fpu_reg(instr.src() as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIV ST(i), ST(0) -- ST(i) = ST(i) / ST(0), pop if instr.b1() & 2
    pub fn fdiv_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = (instr.b1() & 2) != 0;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.dst() as i32) {
            self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            return Ok(());
        }

        let a = self.read_fpu_reg(instr.dst() as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FDIVP ST(i), ST(0) -- same dispatch as fdiv_sti_st0 (pop controlled by b1() & 2)
    pub fn fdivp_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fdiv_sti_st0(instr)
    }

    /// FDIVR ST(i), ST(0) -- ST(i) = ST(0) / ST(i) (reverse), pop if instr.b1() & 2
    pub fn fdivr_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = (instr.b1() & 2) != 0;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.dst() as i32) {
            self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            return Ok(());
        }

        // Reverse: a = ST(0), b = ST(i)
        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(instr.dst() as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FDIVRP ST(i), ST(0) -- same dispatch as fdivr_sti_st0 (pop controlled by b1() & 2)
    pub fn fdivrp_sti_st0(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fdivr_sti_st0(instr)
    }

    /// FDIV single real (f32 from memory) -- ST(0) = ST(0) / f32
    pub fn fdiv_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f32_to_extf80(load_reg, &mut status);
        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIVR single real (f32 from memory) -- ST(0) = f32 / ST(0)
    pub fn fdivr_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f32, b = ST(0)
        let b = self.read_fpu_reg(0);
        let a = f32_to_extf80(load_reg, &mut status);
        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIV double real (f64 from memory) -- ST(0) = ST(0) / f64
    pub fn fdiv_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = f64_to_extf80(load_reg, &mut status);
        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIVR double real (f64 from memory) -- ST(0) = f64 / ST(0)
    pub fn fdivr_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f64, b = ST(0)
        let b = self.read_fpu_reg(0);
        let a = f64_to_extf80(load_reg, &mut status);
        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIDIV word integer (i16 from memory) -- ST(0) = ST(0) / i16
    pub fn fidiv_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg as i32);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIDIVR word integer (i16 from memory) -- ST(0) = i16 / ST(0)
    pub fn fidivr_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        // Reverse: a = i16, b = ST(0)
        let a = i32_to_extf80(load_reg as i32);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIDIV dword integer (i32 from memory) -- ST(0) = ST(0) / i32
    pub fn fidiv_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let a = self.read_fpu_reg(0);
        let b = i32_to_extf80(load_reg);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIDIVRP dword integer (i32 from memory) -- ST(0) = i32 / ST(0)
    pub fn fidivrp_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        // Reverse: a = i32, b = ST(0)
        let a = i32_to_extf80(load_reg);
        let b = self.read_fpu_reg(0);

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_div(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FSQRT, FRNDINT
    // ================================================================

    /// FSQRT -- ST(0) = sqrt(ST(0))
    pub fn fsqrt(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = extf80_sqrt(self.read_fpu_reg(0), &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FRNDINT -- ST(0) = round_to_integer(ST(0))
    pub fn frndint(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rounding_mode = status.softfloat_roundingMode;
        let result = extf80_round_to_int(self.read_fpu_reg(0), rounding_mode, true, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }
}
