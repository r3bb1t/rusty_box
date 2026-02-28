#![allow(dead_code)]
//! FPU load/store instruction handlers.
//!
//! Ported from Bochs `cpu/fpu/fpu_load_store.cc`.
//!
//! All FPU load/store operations:
//! - FLD (single, double, extended, STi)
//! - FILD (word, dword, qword integer)
//! - FBLD (packed BCD)
//! - FST/FSTP (single, double, extended, STi)
//! - FIST/FISTP (word, dword, qword integer)
//! - FISTTP (SSE3 truncation store: word, dword, qword)
//! - FBSTP (packed BCD store)

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::{BxSegregs, Instruction, Opcode};
use super::super::i387::{FPU_CW_INVALID, FPU_EX_STACK_UNDERFLOW};
use super::super::softfloat3e::extf80_to_f32::extf80_to_f32;
use super::super::softfloat3e::extf80_to_f64::extf80_to_f64;
use super::super::softfloat3e::extf80_to_i16::{extf80_to_i16, extf80_to_i16_round_to_zero};
use super::super::softfloat3e::extf80_to_i32::{extf80_to_i32, extf80_to_i32_round_to_zero};
use super::super::softfloat3e::extf80_to_i64::{extf80_to_i64, extf80_to_i64_round_to_zero};
use super::super::softfloat3e::f32_to_extf80::f32_to_extf80;
use super::super::softfloat3e::f64_to_extf80::f64_to_extf80;
use super::super::softfloat3e::i32_to_extf80::i32_to_extf80;
use super::super::softfloat3e::i64_to_extf80::i64_to_extf80;
use super::super::softfloat3e::softfloat::{extf80_sign, floatx80_chs};
use super::super::softfloat3e::softfloat_types::floatx80;
use super::super::softfloat3e::specialize::*;
use super::ferr::i387cw_to_softfloat_status_word;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // read_virtual_qword and write_virtual_qword are defined in access.rs

    // =========================================================================
    // FLD — Load to FPU stack
    // =========================================================================

    /// FLD ST(i) — Push a copy of ST(src) onto the FPU stack.
    ///
    /// If ST(-1) is not empty, stack overflow.
    /// If ST(src) is empty, stack underflow exception; if masked, push default NaN.
    pub fn fld_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
            return Ok(());
        }

        let mut sti_reg = FLOATX80_DEFAULT_NAN;

        if self.is_tag_empty(instr.src() as i32) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            sti_reg = self.read_fpu_reg(instr.src() as i32);
        }

        self.the_i387.fpu_push();
        self.write_fpu_reg(sti_reg, 0);

        Ok(())
    }

    /// FLD m32real — Load single-precision float from memory, convert to
    /// extended precision, and push onto the FPU stack.
    pub fn fld_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = f32_to_extf80(load_reg, &mut status);

        let unmasked = self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false);
        if (unmasked & (FPU_CW_INVALID as u32)) == 0 {
            self.the_i387.fpu_push();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FLD m64real — Load double-precision float from memory, convert to
    /// extended precision, and push onto the FPU stack.
    pub fn fld_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let result = f64_to_extf80(load_reg, &mut status);

        let unmasked = self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false);
        if (unmasked & (FPU_CW_INVALID as u32)) == 0 {
            self.the_i387.fpu_push();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FLD m80real — Load 80-bit extended-precision float from memory
    /// (10 bytes: 8-byte significand + 2-byte sign/exponent) and push.
    pub fn fld_extended_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let signif = self.read_virtual_qword(seg, eaddr)?;
        let sign_exp_addr = eaddr.wrapping_add(8);
        let sign_exp = self.read_virtual_word(seg, sign_exp_addr)?;

        let result = floatx80 { signif, sign_exp };

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            self.the_i387.fpu_push();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // =========================================================================
    // FILD — Load integer to FPU stack
    // =========================================================================

    /// FILD m16int — Load 16-bit signed integer from memory, convert to
    /// extended precision, and push onto the FPU stack.
    pub fn fild_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let result = i32_to_extf80(load_reg as i32);
            self.the_i387.fpu_push();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FILD m32int — Load 32-bit signed integer from memory, convert to
    /// extended precision, and push onto the FPU stack.
    pub fn fild_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let result = i32_to_extf80(load_reg);
            self.the_i387.fpu_push();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    /// FILD m64int — Load 64-bit signed integer from memory, convert to
    /// extended precision, and push onto the FPU stack.
    pub fn fild_qword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_qword(seg, eaddr)? as i64;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
        } else {
            let result = i64_to_extf80(load_reg);
            self.the_i387.fpu_push();
            self.write_fpu_reg(result, 0);
        }

        Ok(())
    }

    // =========================================================================
    // FBLD — Load packed BCD
    // =========================================================================

    /// FBLD m80bcd — Read 10-byte packed BCD from memory, convert to
    /// extended precision float, and push onto the FPU stack.
    ///
    /// Encoding: low 8 bytes = 16 BCD digits (4 bits each, LSB first).
    /// Top 2 bytes: low nibble = 17th digit, next nibble = 18th digit,
    /// bit 15 of the top word = sign bit.
    pub fn fbld_packed_bcd(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let hi2 = self.read_virtual_word(seg, eaddr.wrapping_add(8))?;
        let lo8 = self.read_virtual_qword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if !self.is_tag_empty(-1) {
            self.fpu_stack_overflow(instr);
            return Ok(());
        }

        // Convert packed BCD to 64-bit integer
        let mut scale: i64 = 1;
        let mut val64: i64 = 0;
        let mut lo8_tmp = lo8;

        for _ in 0..16 {
            val64 += ((lo8_tmp & 0x0f) as i64) * scale;
            lo8_tmp >>= 4;
            scale *= 10;
        }

        val64 += ((hi2 & 0x0f) as i64) * scale;
        val64 += (((hi2 >> 4) & 0x0f) as i64) * scale * 10;

        let mut result = i64_to_extf80(val64);
        if (hi2 & 0x8000) != 0 {
            // Negate
            result = floatx80_chs(result);
        }

        self.the_i387.fpu_push();
        self.write_fpu_reg(result, 0);

        Ok(())
    }

    // =========================================================================
    // FST/FSTP — Store from FPU stack (register form)
    // =========================================================================

    /// FST/FSTP ST(i) — Copy ST(0) to ST(dst).
    ///
    /// If FSTP (or FSTP_SPECIAL), also pop the stack.
    /// Opcode `FstpSpecialSti` (D9 D8-DF) behaves like FSTP but does not
    /// raise stack underflow — it just pops silently.
    pub fn fst_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let opcode = instr.get_ia_opcode();
        let pop_stack = opcode != Opcode::FstSti;

        self.clear_c1();

        if self.is_tag_empty(0) {
            // D9 D8-DF: FSTP_SPECIAL — no underflow, just pop
            if opcode == Opcode::FstpSpecialSti {
                self.the_i387.fpu_pop();
            } else {
                self.fpu_stack_underflow(instr, instr.dst() as i32, pop_stack);
            }
        } else {
            let st0_reg = self.read_fpu_reg(0);

            self.write_fpu_reg(st0_reg, instr.dst() as i32);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    // =========================================================================
    // FST/FSTP — Store from FPU stack (single-precision memory form)
    // =========================================================================

    /// FST/FSTP m32real — Convert ST(0) to single-precision float and store
    /// to memory.  FSTP also pops the stack.
    pub fn fst_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        self.clear_c1();

        let mut save_reg: u32 = FLOAT32_DEFAULT_NAN; // masked response

        let pop_stack = instr.get_ia_opcode() == Opcode::FstpSingleReal;

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_f32(self.read_fpu_reg(0), &mut status);

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Store to memory might generate an exception; in that case
        // the original FPU status word must be preserved.
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_dword(seg, eaddr, save_reg)?;

        self.the_i387.swd = saved_swd;
        if pop_stack {
            self.the_i387.fpu_pop();
        }

        Ok(())
    }

    /// FSTP m32real — same handler as fst_single_real (pop determined by opcode).
    pub fn fstp_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fst_single_real(instr)
    }

    // =========================================================================
    // FST/FSTP — Store from FPU stack (double-precision memory form)
    // =========================================================================

    /// FST/FSTP m64real — Convert ST(0) to double-precision float and store
    /// to memory.  FSTP also pops the stack.
    pub fn fst_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        self.clear_c1();

        let mut save_reg: u64 = FLOAT64_DEFAULT_NAN; // masked response

        let pop_stack = instr.get_ia_opcode() == Opcode::FstpDoubleReal;

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_f64(self.read_fpu_reg(0), &mut status);

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_qword(seg, eaddr, save_reg)?;

        self.the_i387.swd = saved_swd;
        if pop_stack {
            self.the_i387.fpu_pop();
        }

        Ok(())
    }

    /// FSTP m64real — same handler as fst_double_real (pop determined by opcode).
    pub fn fstp_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fst_double_real(instr)
    }

    // =========================================================================
    // FSTP — Store extended real (always pops)
    // =========================================================================

    /// FSTP m80real — Store ST(0) as 80-bit extended-precision float to memory
    /// (10 bytes: 8-byte significand + 2-byte sign/exponent).  Always pops.
    pub fn fstp_extended_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        let mut save_reg = FLOATX80_DEFAULT_NAN; // masked response

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            save_reg = self.read_fpu_reg(0);
        }

        self.write_virtual_qword(seg, eaddr, save_reg.signif)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(8), save_reg.sign_exp)?;

        self.the_i387.fpu_pop();

        Ok(())
    }

    // =========================================================================
    // FIST/FISTP — Store as integer (word)
    // =========================================================================

    /// FIST/FISTP m16int — Convert ST(0) to 16-bit signed integer and store
    /// to memory.  FISTP also pops the stack.
    pub fn fist_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        let mut save_reg: i16 = INT16_INDEFINITE;

        let pop_stack = instr.get_ia_opcode() == Opcode::FistpWordInteger;

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_i16(self.read_fpu_reg(0), &mut status);

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_word(seg, eaddr, save_reg as u16)?;

        self.the_i387.swd = saved_swd;
        if pop_stack {
            self.the_i387.fpu_pop();
        }

        Ok(())
    }

    /// FISTP m16int — same handler as fist_word_integer (pop determined by opcode).
    pub fn fistp_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fist_word_integer(instr)
    }

    // =========================================================================
    // FIST/FISTP — Store as integer (dword)
    // =========================================================================

    /// FIST/FISTP m32int — Convert ST(0) to 32-bit signed integer and store
    /// to memory.  FISTP also pops the stack.
    pub fn fist_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        let mut save_reg: i32 = INT32_INDEFINITE;

        let pop_stack = instr.get_ia_opcode() == Opcode::FistpDwordInteger;

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_i32(
                self.read_fpu_reg(0),
                status.softfloat_roundingMode,
                true,
                &mut status,
            );

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_dword(seg, eaddr, save_reg as u32)?;

        self.the_i387.swd = saved_swd;
        if pop_stack {
            self.the_i387.fpu_pop();
        }

        Ok(())
    }

    /// FISTP m32int — same handler as fist_dword_integer (pop determined by opcode).
    pub fn fistp_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fist_dword_integer(instr)
    }

    // =========================================================================
    // FISTP — Store as integer (qword, always pops)
    // =========================================================================

    /// FISTP m64int — Convert ST(0) to 64-bit signed integer and store
    /// to memory.  Always pops the stack.
    pub fn fistp_qword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        let mut save_reg: i64 = INT64_INDEFINITE;

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_i64(
                self.read_fpu_reg(0),
                status.softfloat_roundingMode,
                true,
                &mut status,
            );

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_qword(seg, eaddr, save_reg as u64)?;

        self.the_i387.swd = saved_swd;

        self.the_i387.fpu_pop();

        Ok(())
    }

    // =========================================================================
    // FBSTP — Store packed BCD (always pops)
    // =========================================================================

    /// FBSTP m80bcd — Convert ST(0) to packed BCD and store 10 bytes to memory.
    /// Always pops the stack.
    ///
    /// The packed BCD integer indefinite encoding (FFFFC000000000000000h)
    /// is stored in response to a masked floating-point invalid-operation
    /// exception.
    pub fn fbstp_packed_bcd(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        // Packed BCD indefinite: the masked response
        let mut save_reg_hi: u16 = 0xFFFF;
        let mut save_reg_lo: u64 = 0xC000000000000000;

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            let reg = self.read_fpu_reg(0);
            let mut save_val = extf80_to_i64(reg, status.softfloat_roundingMode, true, &mut status);

            let sign = extf80_sign(reg);
            if sign {
                save_val = -save_val;
            }

            if save_val > 999_999_999_999_999_999i64 {
                // throw away other flags — only invalid matters
                super::super::softfloat3e::softfloat::softfloat_setFlags(
                    &mut status,
                    super::super::softfloat3e::softfloat::FLAG_INVALID,
                );
            }

            if (status.softfloat_exceptionFlags
                & super::super::softfloat3e::softfloat::FLAG_INVALID)
                == 0
            {
                save_reg_hi = if sign { 0x8000 } else { 0 };
                save_reg_lo = 0;

                for i in 0..16 {
                    save_reg_lo += ((save_val % 10) as u64) << (4 * i);
                    save_val /= 10;
                }

                save_reg_hi += (save_val % 10) as u16;
                save_val /= 10;
                save_reg_hi += ((save_val % 10) as u16) << 4;
            }

            // Check for FPU arithmetic exceptions
            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        // Write packed BCD to memory
        self.write_virtual_qword(seg, eaddr, save_reg_lo)?;
        self.write_virtual_word(seg, eaddr.wrapping_add(8), save_reg_hi)?;

        self.the_i387.swd = saved_swd;

        self.the_i387.fpu_pop();

        Ok(())
    }

    // =========================================================================
    // FISTTP — SSE3 truncation store (always pops)
    // =========================================================================

    /// FISTTP m16int — Convert ST(0) to 16-bit integer using round-to-zero
    /// (truncation), store to memory, and pop.  SSE3 instruction.
    pub fn fisttp16(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        let mut save_reg: i16 = INT16_INDEFINITE; // masked response

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_i16_round_to_zero(self.read_fpu_reg(0), &mut status);

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_word(seg, eaddr, save_reg as u16)?;

        self.the_i387.swd = saved_swd;

        self.the_i387.fpu_pop();

        Ok(())
    }

    /// FISTTP m32int — Convert ST(0) to 32-bit integer using round-to-zero
    /// (truncation), store to memory, and pop.  SSE3 instruction.
    pub fn fisttp32(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        let mut save_reg: i32 = INT32_INDEFINITE; // masked response

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_i32_round_to_zero(self.read_fpu_reg(0), true, &mut status);

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_dword(seg, eaddr, save_reg as u32)?;

        self.the_i387.swd = saved_swd;

        self.the_i387.fpu_pop();

        Ok(())
    }

    /// FISTTP m64int — Convert ST(0) to 64-bit integer using round-to-zero
    /// (truncation), store to memory, and pop.  SSE3 instruction.
    pub fn fisttp64(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.fpu_update_last_instruction(instr);

        let x87_sw = self.the_i387.swd;

        let mut save_reg: i64 = INT64_INDEFINITE; // masked response

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);

            if !self.the_i387.is_ia_masked() {
                return Ok(());
            }
        } else {
            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            save_reg = extf80_to_i64_round_to_zero(self.read_fpu_reg(0), true, &mut status);

            if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, true) != 0 {
                return Ok(());
            }
        }

        // Save/restore status word around memory write
        let saved_swd = self.the_i387.swd;
        self.the_i387.swd = x87_sw;

        self.write_virtual_qword(seg, eaddr, save_reg as u64)?;

        self.the_i387.swd = saved_swd;

        self.the_i387.fpu_pop();

        Ok(())
    }
}
