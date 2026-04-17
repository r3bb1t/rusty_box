#![allow(dead_code)]
//! FPU arithmetic instructions: FADD, FMUL, FSUB, FSUBR, FDIV, FDIVR, FSQRT, FRNDINT
//! Ported from Bochs cpu/fpu/fpu_arith.cc

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::{BxSegregs, Instruction};
use super::super::softfloat3e::extf80_addsub::{extf80_add, extf80_sub};
use super::super::softfloat3e::extf80_div::extf80_div;
use super::super::softfloat3e::extf80_mul::extf80_mul;
use super::super::softfloat3e::extf80_roundToInt::extf80_round_to_int;
use super::super::softfloat3e::extf80_sqrt::extf80_sqrt;
use super::super::softfloat3e::f32_to_extf80::f32_to_extf80;
use super::super::softfloat3e::f64_to_extf80::f64_to_extf80;
use super::super::softfloat3e::i32_to_extf80::i32_to_extf80;
use super::super::softfloat3e::softfloat::{
    extf80_is_nan, extf80_is_signaling_nan, extf80_is_unsupported, f32_is_nan, f32_is_signaling_nan,
    f64_is_nan, f64_is_signaling_nan, softfloat_raiseFlags, SoftFloatStatus, FLAG_INVALID,
};
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::{softfloat_propagate_nan_extf80, FLOATX80_DEFAULT_NAN};
use super::ferr::i387cw_to_softfloat_status_word;

// ================================================================
// FPU_handle_NaN helpers for memory-form arithmetic
// Matches Bochs fpu_arith.cc lines 69-165
// ================================================================

/// Inner NaN propagation for extf80 vs f32.
/// Matches Bochs `FPU_handle_NaN(floatx80 a, int aIsNaN, float32 b32, int bIsNaN, status)`.
fn fpu_handle_nan_inner_f32(
    a: floatx80,
    a_is_nan: bool,
    b32: u32,
    b_is_nan: bool,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let a_is_signaling_nan = extf80_is_signaling_nan(a);
    let b_is_signaling_nan = f32_is_signaling_nan(b32);

    if a_is_signaling_nan | b_is_signaling_nan {
        softfloat_raiseFlags(status, FLAG_INVALID);
    }

    // Propagate QNaN from SNaN for a
    let a_q = softfloat_propagate_nan_extf80(a.sign_exp, a.signif, 0, 0, status);

    if a_is_nan & !b_is_nan {
        return a_q;
    }

    // float32 is NaN so conversion will propagate SNaN to QNaN and raise
    // appropriate exception flags
    let b = f32_to_extf80(b32, status);

    if a_is_signaling_nan {
        if b_is_signaling_nan {
            // Both signaling: return larger significand
            if a_q.signif < b.signif {
                return b;
            }
            if b.signif < a_q.signif {
                return a_q;
            }
            return if a_q.sign_exp < b.sign_exp { a_q } else { b };
        }
        if b_is_nan { b } else { a_q }
    } else if a_is_nan {
        if b_is_signaling_nan {
            return a_q;
        }
        // Both are quiet NaN: return larger significand
        if a_q.signif < b.signif {
            return b;
        }
        if b.signif < a_q.signif {
            return a_q;
        }
        if a_q.sign_exp < b.sign_exp { a_q } else { b }
    } else {
        // Only b is NaN
        b
    }
}

/// Check and handle NaN for extf80 vs f32 memory operand.
/// Matches Bochs `bool FPU_handle_NaN(floatx80 a, float32 b, floatx80 &r, status)`.
/// Returns `Some(result)` if NaN was handled, `None` if normal arithmetic should proceed.
fn fpu_handle_nan_f32(a: floatx80, b: u32, status: &mut SoftFloatStatus) -> Option<floatx80> {
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return Some(FLOATX80_DEFAULT_NAN);
    }

    let a_is_nan = extf80_is_nan(a);
    let b_is_nan = f32_is_nan(b);
    if a_is_nan | b_is_nan {
        return Some(fpu_handle_nan_inner_f32(a, a_is_nan, b, b_is_nan, status));
    }
    None
}

/// Inner NaN propagation for extf80 vs f64.
/// Matches Bochs `FPU_handle_NaN(floatx80 a, int aIsNaN, float64 b64, int bIsNaN, status)`.
fn fpu_handle_nan_inner_f64(
    a: floatx80,
    a_is_nan: bool,
    b64: u64,
    b_is_nan: bool,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let a_is_signaling_nan = extf80_is_signaling_nan(a);
    let b_is_signaling_nan = f64_is_signaling_nan(b64);

    if a_is_signaling_nan | b_is_signaling_nan {
        softfloat_raiseFlags(status, FLAG_INVALID);
    }

    // Propagate QNaN from SNaN for a
    let a_q = softfloat_propagate_nan_extf80(a.sign_exp, a.signif, 0, 0, status);

    if a_is_nan & !b_is_nan {
        return a_q;
    }

    // float64 is NaN so conversion will propagate SNaN to QNaN and raise
    // appropriate exception flags
    let b = f64_to_extf80(b64, status);

    if a_is_signaling_nan {
        if b_is_signaling_nan {
            // Both signaling: return larger significand
            if a_q.signif < b.signif {
                return b;
            }
            if b.signif < a_q.signif {
                return a_q;
            }
            return if a_q.sign_exp < b.sign_exp { a_q } else { b };
        }
        if b_is_nan { b } else { a_q }
    } else if a_is_nan {
        if b_is_signaling_nan {
            return a_q;
        }
        // Both are quiet NaN: return larger significand
        if a_q.signif < b.signif {
            return b;
        }
        if b.signif < a_q.signif {
            return a_q;
        }
        if a_q.sign_exp < b.sign_exp { a_q } else { b }
    } else {
        // Only b is NaN
        b
    }
}

/// Check and handle NaN for extf80 vs f64 memory operand.
/// Matches Bochs `bool FPU_handle_NaN(floatx80 a, float64 b, floatx80 &r, status)`.
/// Returns `Some(result)` if NaN was handled, `None` if normal arithmetic should proceed.
fn fpu_handle_nan_f64(a: floatx80, b: u64, status: &mut SoftFloatStatus) -> Option<floatx80> {
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return Some(FLOATX80_DEFAULT_NAN);
    }

    let a_is_nan = extf80_is_nan(a);
    let b_is_nan = f64_is_nan(b);
    if a_is_nan | b_is_nan {
        return Some(fpu_handle_nan_inner_f64(a, a_is_nan, b, b_is_nan, status));
    }
    None
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ================================================================
    // FADD variants
    // ================================================================

    /// FADD ST(0), ST(j) -- ST(0) = ST(0) + ST(j)
    pub fn fadd_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f32(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_add(a, f32_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FADD double real (f64 from memory)
    pub fn fadd_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f64(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_add(a, f64_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIADD word integer (i16 from memory)
    pub fn fiadd_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_word(seg, eaddr)? as i16;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)? as i32;

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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f32(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_mul(a, f32_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FMUL double real (f64 from memory)
    pub fn fmul_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f64(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_mul(a, f64_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIMUL word integer (i16 from memory)
    pub fn fimul_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_word(seg, eaddr)? as i16;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)? as i32;

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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f32(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_sub(a, f32_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUBR single real (f32 from memory) -- ST(0) = f32 - ST(0)
    pub fn fsubr_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f32, b = ST(0)
        let b = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f32(b, load_reg, &mut status) {
            nan_result
        } else {
            extf80_sub(f32_to_extf80(load_reg, &mut status), b, &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUB double real (f64 from memory) -- ST(0) = ST(0) - f64
    pub fn fsub_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f64(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_sub(a, f64_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FSUBR double real (f64 from memory) -- ST(0) = f64 - ST(0)
    pub fn fsubr_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f64, b = ST(0)
        let b = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f64(b, load_reg, &mut status) {
            nan_result
        } else {
            extf80_sub(f64_to_extf80(load_reg, &mut status), b, &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FISUB word integer (i16 from memory) -- ST(0) = ST(0) - i16
    pub fn fisub_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_word(seg, eaddr)? as i16;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_word(seg, eaddr)? as i16;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)? as i32;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)? as i32;

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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f32(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_div(a, f32_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIVR single real (f32 from memory) -- ST(0) = f32 / ST(0)
    pub fn fdivr_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f32, b = ST(0)
        let b = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f32(b, load_reg, &mut status) {
            nan_result
        } else {
            extf80_div(f32_to_extf80(load_reg, &mut status), b, &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIV double real (f64 from memory) -- ST(0) = ST(0) / f64
    pub fn fdiv_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f64(a, load_reg, &mut status) {
            nan_result
        } else {
            extf80_div(a, f64_to_extf80(load_reg, &mut status), &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FDIVR double real (f64 from memory) -- ST(0) = f64 / ST(0)
    pub fn fdivr_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        // Reverse: a = f64, b = ST(0)
        let b = self.read_fpu_reg(0);
        let result = if let Some(nan_result) = fpu_handle_nan_f64(b, load_reg, &mut status) {
            nan_result
        } else {
            extf80_div(f64_to_extf80(load_reg, &mut status), b, &mut status)
        };

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FIDIV word integer (i16 from memory) -- ST(0) = ST(0) / i16
    pub fn fidiv_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_word(seg, eaddr)? as i16;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_word(seg, eaddr)? as i16;

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
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)? as i32;

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
    pub fn fidivr_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.v_read_dword(seg, eaddr)? as i32;

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
        self.fpu_check_pending_exceptions()?;
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
        self.fpu_check_pending_exceptions()?;
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
