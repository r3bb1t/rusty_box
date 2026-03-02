#![allow(dead_code)]
//! FPU transcendental instruction handlers: FSCALE, FXTRACT, FPREM, FPREM1,
//! F2XM1, FYL2X, FYL2XP1, FPTAN, FPATAN, FSIN, FCOS, FSINCOS
//! Ported from Bochs cpu/fpu/fpu_trans.cc.
//!
//! The actual implementations are in separate files matching the Bochs layout:
//! - fprem.rs: FPREM/FPREM1 remainder (fprem.cc)
//! - f2xm1.rs: F2XM1 exponential (f2xm1.cc)
//! - fyl2x.rs: FYL2X/FYL2XP1 logarithm (fyl2x.cc)
//! - fpatan.rs: FPATAN arctangent (fpatan.cc)
//! - fsincos.rs: FSIN/FCOS/FSINCOS/FPTAN trigonometric (fsincos.cc)

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::Instruction;
use super::super::i387::*;
use super::super::softfloat3e::extf80_scale::extf80_scale;
use super::super::softfloat3e::i32_to_extf80::i32_to_extf80;
use super::super::softfloat3e::internals::*;
use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::*;
use super::ferr::i387cw_to_softfloat_status_word;

// Import implementation functions from split files
use super::f2xm1::f2xm1_impl;
use super::fpatan::fpatan_impl;
use super::fprem::do_fprem;
use super::fsincos::{
    fsincos_both, fsincos_single, ftan_impl, FtanResult, SinCosBothResult, SinCosResult,
};
use super::fyl2x::{fyl2x_impl, fyl2xp1_impl};

/// 1.0 in floatx80 format
const FLOATX80_ONE: floatx80 = floatx80 {
    signif: 0x8000000000000000,
    sign_exp: 0x3FFF,
};

// ---------------------------------------------------------------------------
// CPU methods: FPU transcendental instructions
// ---------------------------------------------------------------------------

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ================================================================
    // FSCALE (D9 FD) -- ST(0) = ST(0) * 2^trunc(ST(1))
    // ================================================================

    /// FSCALE -- Scale ST(0) by power of 2 stored in ST(1).
    /// Ported from Bochs fpu_trans.cc FSCALE.
    pub fn fscale(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(1);
        let result = extf80_scale(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FXTRACT (D9 F4) -- Extract exponent and significand
    // ================================================================

    /// FXTRACT -- Extract exponent and significand of ST(0).
    /// Pushes significand onto stack, stores exponent in old ST(0).
    /// After: ST(0) = significand, ST(1) = exponent.
    /// Ported from Bochs fpu_trans.cc FXTRACT.
    pub fn fxtract(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || !self.is_tag_empty(-1) {
            if self.is_tag_empty(0) {
                self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            } else {
                self.fpu_exception(instr, FPU_EX_STACK_OVERFLOW as u32, false);
            }
            if self.the_i387.is_ia_masked() {
                self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
                self.the_i387.fpu_push();
                self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
            }
            return Ok(());
        }

        // Bochs fpu_trans.cc: FXTRACT uses CW as-is (no forced 80-bit precision)
        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let (significand, exponent) = extf80_extract_impl(a, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            // ST(0) = exponent
            self.write_fpu_reg(exponent, 0);
            // Push significand (becomes new ST(0))
            self.the_i387.fpu_push();
            self.write_fpu_reg(significand, 0);
        }

        Ok(())
    }

    // ================================================================
    // FPREM (D9 F8) -- Partial remainder (truncation)
    // ================================================================

    /// FPREM -- IEEE partial remainder using truncation (round-to-zero).
    /// Ported from Bochs fpu_trans.cc FPREM and fprem.cc.
    pub fn fprem(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.setcc(0); // clear C2

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(1);
        let mut result = floatx80::default();
        let mut quotient: u64 = 0;

        let flags = do_fprem(a, b, &mut result, &mut quotient, ROUND_TO_ZERO, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if flags >= 0 {
                let mut cc: u16 = 0;
                if flags != 0 {
                    cc = FPU_SW_C2;
                } else {
                    if quotient & 1 != 0 {
                        cc |= FPU_SW_C1;
                    }
                    if quotient & 2 != 0 {
                        cc |= FPU_SW_C3;
                    }
                    if quotient & 4 != 0 {
                        cc |= FPU_SW_C0;
                    }
                }
                self.setcc(cc);
            }
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FPREM1 (D9 F5) -- Partial remainder (round-to-nearest)
    // ================================================================

    /// FPREM1 -- IEEE partial remainder using round-to-nearest.
    /// Ported from Bochs fpu_trans.cc FPREM1 and fprem.cc.
    pub fn fprem1(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.setcc(0); // clear C2

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);
        let b = self.read_fpu_reg(1);
        let mut result = floatx80::default();
        let mut quotient: u64 = 0;

        let flags = do_fprem(
            a,
            b,
            &mut result,
            &mut quotient,
            ROUND_NEAR_EVEN,
            &mut status,
        );

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if flags >= 0 {
                let mut cc: u16 = 0;
                if flags != 0 {
                    cc = FPU_SW_C2;
                } else {
                    if quotient & 1 != 0 {
                        cc |= FPU_SW_C1;
                    }
                    if quotient & 2 != 0 {
                        cc |= FPU_SW_C3;
                    }
                    if quotient & 4 != 0 {
                        cc |= FPU_SW_C0;
                    }
                }
                self.setcc(cc);
            }
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // F2XM1 (D9 F0) -- Compute 2^x - 1
    // ================================================================

    /// F2XM1 -- Compute 2^ST(0) - 1, where -1 <= ST(0) <= 1.
    /// Uses Float128 polynomial evaluation matching Bochs f2xm1.cc.
    pub fn f2xm1(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0);
        let result = f2xm1_impl(a, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FYL2X (D9 F1) -- Compute y * log2(x)
    // ================================================================

    /// FYL2X -- Compute ST(1) * log2(ST(0)), store result in ST(1), pop ST(0).
    /// Uses Float128 polynomial evaluation matching Bochs fyl2x.cc.
    pub fn fyl2x(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_stack_underflow(instr, 1, true /* pop_stack */);
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0); // x
        let b = self.read_fpu_reg(1); // y
        let result = fyl2x_impl(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.the_i387.fpu_pop();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FYL2XP1 (D9 F9) -- Compute y * log2(x + 1)
    // ================================================================

    /// FYL2XP1 -- Compute ST(1) * log2(ST(0) + 1), store result in ST(1), pop ST(0).
    /// Uses Float128 polynomial evaluation matching Bochs fyl2x.cc.
    pub fn fyl2xp1(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_stack_underflow(instr, 1, true /* pop_stack */);
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0); // x
        let b = self.read_fpu_reg(1); // y
        let result = fyl2xp1_impl(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.the_i387.fpu_pop();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FPTAN (D9 F2) -- Partial tangent
    // ================================================================

    /// FPTAN -- Compute tangent of ST(0), replace ST(0) with result, push 1.0.
    /// Sets C2=1 if argument is out of range (|x| >= 2^63).
    /// Uses Float128 polynomial evaluation matching Bochs fsincos.cc ftan().
    pub fn fptan(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        // Clear C2 initially (will be set if out of range)
        self.the_i387.swd &= !FPU_SW_C2;

        if self.is_tag_empty(0) || !self.is_tag_empty(-1) {
            if self.is_tag_empty(0) {
                self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            } else {
                self.fpu_exception(instr, FPU_EX_STACK_OVERFLOW as u32, false);
            }
            // Masked response
            if self.the_i387.is_ia_masked() {
                self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
                self.the_i387.fpu_push();
                self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
            }
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0);
        let tan_result = ftan_impl(a, &mut status);

        match tan_result {
            FtanResult::OutOfRange => {
                // Set C2 = 1 to indicate argument out of range
                self.the_i387.swd |= FPU_SW_C2;
            }
            FtanResult::Nan(y) => {
                if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
                    self.write_fpu_reg(y, 0);
                    self.the_i387.fpu_push();
                    self.write_fpu_reg(y, 0);
                }
            }
            FtanResult::Value(y) => {
                if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
                    self.write_fpu_reg(y, 0);
                    self.the_i387.fpu_push();
                    self.write_fpu_reg(FLOATX80_ONE, 0);
                }
            }
        }

        Ok(())
    }

    // ================================================================
    // FPATAN (D9 F3) -- Partial arctangent
    // ================================================================

    /// FPATAN -- Compute atan2(ST(1), ST(0)), store in ST(1), pop ST(0).
    /// Result = atan(ST(1)/ST(0)).
    /// Uses Float128 polynomial evaluation matching Bochs fpatan.cc.
    pub fn fpatan(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_stack_underflow(instr, 1, true /* pop_stack */);
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0); // x (divisor)
        let b = self.read_fpu_reg(1); // y (dividend)
        let result = fpatan_impl(a, b, &mut status);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.the_i387.fpu_pop();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // ================================================================
    // FSIN (D9 FE) -- Sine
    // ================================================================

    /// FSIN -- Compute sine of ST(0).
    /// Sets C2=1 if argument is out of range (|x| >= 2^63).
    /// Uses Float128 polynomial evaluation matching Bochs fsincos.cc.
    pub fn fsin(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387.swd &= !FPU_SW_C2;

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0);
        let result = fsincos_single(a, true /* want_sin */, &mut status);

        match result {
            SinCosResult::OutOfRange => {
                self.the_i387.swd |= FPU_SW_C2;
            }
            SinCosResult::Value(y) => {
                if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
                    self.write_fpu_reg(y, 0);
                }
            }
        }

        Ok(())
    }

    // ================================================================
    // FCOS (D9 FF) -- Cosine
    // ================================================================

    /// FCOS -- Compute cosine of ST(0).
    /// Sets C2=1 if argument is out of range (|x| >= 2^63).
    /// Uses Float128 polynomial evaluation matching Bochs fsincos.cc.
    pub fn fcos(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387.swd &= !FPU_SW_C2;

        if self.is_tag_empty(0) {
            self.fpu_stack_underflow(instr, 0, false);
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0);
        let result = fsincos_single(a, false /* want_cos */, &mut status);

        match result {
            SinCosResult::OutOfRange => {
                self.the_i387.swd |= FPU_SW_C2;
            }
            SinCosResult::Value(y) => {
                if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
                    self.write_fpu_reg(y, 0);
                }
            }
        }

        Ok(())
    }

    // ================================================================
    // FSINCOS (D9 FB) -- Sine and cosine
    // ================================================================

    /// FSINCOS -- Compute sin(ST(0)) and cos(ST(0)).
    /// sin goes to old ST(0), cos is pushed onto stack (becomes new ST(0)).
    /// Actually: after operation, ST(0)=cos, ST(1)=sin per Bochs.
    /// Bochs: BX_WRITE_FPU_REG(sin_y, 0); FPU_push(); BX_WRITE_FPU_REG(cos_y, 0);
    /// Sets C2=1 if argument is out of range (|x| >= 2^63).
    /// Uses Float128 polynomial evaluation matching Bochs fsincos.cc.
    pub fn fsincos(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);

        self.clear_c1();
        self.the_i387.swd &= !FPU_SW_C2;

        if self.is_tag_empty(0) || !self.is_tag_empty(-1) {
            if self.is_tag_empty(0) {
                self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            } else {
                self.fpu_exception(instr, FPU_EX_STACK_OVERFLOW as u32, false);
            }
            // Masked response
            if self.the_i387.is_ia_masked() {
                self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
                self.the_i387.fpu_push();
                self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
            }
            return Ok(());
        }

        let mut status =
            i387cw_to_softfloat_status_word(self.the_i387.get_control_word() | FPU_PR_80_BITS);

        let a = self.read_fpu_reg(0);
        let result = fsincos_both(a, &mut status);

        match result {
            SinCosBothResult::OutOfRange => {
                self.the_i387.swd |= FPU_SW_C2;
            }
            SinCosBothResult::Values(sin_y, cos_y) => {
                if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
                    self.write_fpu_reg(sin_y, 0);
                    self.the_i387.fpu_push();
                    self.write_fpu_reg(cos_y, 0);
                }
            }
        }

        Ok(())
    }
}

// ===========================================================================
// Internal implementation functions (not methods on BxCpuC)
// ===========================================================================

// ---------------------------------------------------------------------------
// extf80_extract (FXTRACT helper)
// ---------------------------------------------------------------------------

/// Extract exponent and significand from a floatx80 value.
/// Returns (significand, exponent) as floatx80 values.
/// Ported from Bochs softfloat3e extF80_extract.c.
fn extf80_extract_impl(a: floatx80, status: &mut SoftFloatStatus) -> (floatx80, floatx80) {
    // Handle unsupported encodings
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        let nan = FLOATX80_DEFAULT_NAN;
        return (nan, nan);
    }

    let sign_a = sign_extf80(a.sign_exp);
    let mut exp_a = exp_extf80(a.sign_exp) as i32;
    let mut sig_a = a.signif;

    // Infinity or NaN
    if exp_a == 0x7FFF {
        if (sig_a << 1) != 0 {
            // NaN: propagate
            let propagated = softfloat_propagate_nan_extf80(a.sign_exp, sig_a, 0, 0, status);
            return (propagated, propagated);
        }
        // Infinity: exponent = +inf, significand = a (unchanged)
        let exp_val = pack_floatx80(false, 0x7FFF, 0x8000000000000000);
        return (a, exp_val);
    }

    // Zero or denormal
    if exp_a == 0 {
        if sig_a == 0 {
            // Zero: raise divide-by-zero, return significand = signed zero, exponent = -inf
            softfloat_raiseFlags(status, FLAG_DIVBYZERO);
            let significand = pack_floatx80(sign_a, 0, 0);
            let exponent = pack_floatx80(true, 0x7FFF, 0x8000000000000000); // -inf
            return (significand, exponent);
        }
        // Denormal: normalize
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a = norm.exp + 1;
        sig_a = norm.sig;
    }

    // Normal number: significand has sign preserved with exponent = bias (1.xxx form)
    let significand = pack_floatx80(sign_a, 0x3FFF, sig_a);
    // Exponent is (biased_exponent - bias) as a float
    let exponent = i32_to_extf80(exp_a - 0x3FFF);

    (significand, exponent)
}
