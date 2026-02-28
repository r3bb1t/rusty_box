#![allow(dead_code)]
//! FPU comparison instruction handlers for x86 CPU emulation.
//!
//! Ported from Bochs `cpu/fpu/fpu_compare.cc`.
//!
//! Implements:
//! - FCOM/FCOMP ST(i): compare ST(0) with ST(src), set condition codes
//! - FUCOM/FUCOMP ST(i): unordered compare (quiet — only SNaN raises invalid)
//! - FCOM/FCOMP single/double real: compare ST(0) with f32/f64 memory operand
//! - FICOM/FICOMP word/dword integer: compare ST(0) with i16/i32 memory operand
//! - FCOMPP/FUCOMPP: compare and double-pop
//! - FCOMI/FCOMIP ST(0),ST(j): compare, write result to EFLAGS
//! - FUCOMI/FUCOMIP ST(0),ST(j): unordered compare, write result to EFLAGS
//! - FTST: compare ST(0) with +0.0
//! - FXAM: examine ST(0) and set C3/C2/C1/C0 based on class

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::{BxSegregs, Instruction, Opcode};
use super::super::i387::*;
use super::super::softfloat3e::extf80_class::extf80_class;
use super::super::softfloat3e::extf80_compare::extf80_compare;
use super::super::softfloat3e::f32_to_extf80::f32_to_extf80;
use super::super::softfloat3e::f64_to_extf80::f64_to_extf80;
use super::super::softfloat3e::i32_to_extf80::i32_to_extf80;
use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::specialize::*;
use super::ferr::i387cw_to_softfloat_status_word;

// ---------------------------------------------------------------------------
// Helper: convert softfloat relation to FPU status word condition-code flags
// ---------------------------------------------------------------------------

/// Convert a softfloat comparison relation to the FPU status word
/// condition-code flags (C0, C2, C3).
///
/// Mapping:
/// - UNORDERED: C0 | C2 | C3
/// - GREATER:   0
/// - LESS:      C0
/// - EQUAL:     C3
fn status_word_flags_fpu_compare(float_relation: i32) -> u16 {
    match float_relation {
        RELATION_UNORDERED => FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3,
        RELATION_GREATER => 0,
        RELATION_LESS => FPU_SW_C0,
        RELATION_EQUAL => FPU_SW_C3,
        _ => FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3, // treat unknown as unordered
    }
}

// ---------------------------------------------------------------------------
// Instruction handlers
// ---------------------------------------------------------------------------

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =======================================================================
    // FCOM ST(i) / FCOMP ST(i)
    // =======================================================================

    /// FCOM ST(i) — Compare ST(0) with ST(src), set FPU condition codes.
    /// Also used for FCOMP (pops ST(0) after comparison).
    pub fn fcom_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = instr.get_ia_opcode() == Opcode::FcompSti;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            self.read_fpu_reg(instr.src() as i32),
            false,
            &mut status,
        );
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FCOMP ST(i) — Compare ST(0) with ST(src) and pop.
    /// Dispatches to fcom_sti (pop determined by opcode).
    pub fn fcomp_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fcom_sti(instr)
    }

    // =======================================================================
    // FUCOM ST(i) / FUCOMP ST(i)
    // =======================================================================

    /// FUCOM ST(i) — Unordered compare ST(0) with ST(src).
    /// Quiet comparison: only SNaN raises invalid (QNaN does not).
    /// Also used for FUCOMP (pops after comparison).
    pub fn fucom_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = instr.get_ia_opcode() == Opcode::FucompSti;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            self.read_fpu_reg(instr.src() as i32),
            true, // quiet
            &mut status,
        );
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FUCOMP ST(i) — Unordered compare and pop.
    /// Dispatches to fucom_sti (pop determined by opcode).
    pub fn fucomp_sti(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fucom_sti(instr)
    }

    // =======================================================================
    // FCOM / FCOMP single-real (f32 memory operand)
    // =======================================================================

    /// FCOM single-real — Compare ST(0) with a 32-bit float from memory.
    /// Also used for FCOMP single-real (pops after comparison).
    pub fn fcom_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let pop_stack = instr.get_ia_opcode() == Opcode::FcompSingleReal;

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)?;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);

        // Bochs manually checks for NaN before comparing for memory operands
        let rc = if extf80_is_nan(a) || extf80_is_unsupported(a) || f32_is_nan(load_reg) {
            softfloat_raiseFlags(&mut status, FLAG_INVALID);
            RELATION_UNORDERED
        } else {
            extf80_compare(a, f32_to_extf80(load_reg, &mut status), false, &mut status)
        };
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FCOMP single-real — Compare ST(0) with f32 from memory and pop.
    /// Dispatches to fcom_single_real (pop determined by opcode).
    pub fn fcomp_single_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fcom_single_real(instr)
    }

    // =======================================================================
    // FCOM / FCOMP double-real (f64 memory operand)
    // =======================================================================

    /// FCOM double-real — Compare ST(0) with a 64-bit float from memory.
    /// Also used for FCOMP double-real (pops after comparison).
    pub fn fcom_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let pop_stack = instr.get_ia_opcode() == Opcode::FcompDoubleReal;

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        // Read 64-bit float as two 32-bit halves (little-endian)
        let lo = self.read_virtual_dword(seg, eaddr)? as u64;
        let hi = self.read_virtual_dword(seg, eaddr.wrapping_add(4))? as u64;
        let load_reg = lo | (hi << 32);

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let a = self.read_fpu_reg(0);

        // Bochs manually checks for NaN before comparing for memory operands
        let rc = if extf80_is_nan(a) || extf80_is_unsupported(a) || f64_is_nan(load_reg) {
            softfloat_raiseFlags(&mut status, FLAG_INVALID);
            RELATION_UNORDERED
        } else {
            extf80_compare(a, f64_to_extf80(load_reg, &mut status), false, &mut status)
        };
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FCOMP double-real — Compare ST(0) with f64 from memory and pop.
    /// Dispatches to fcom_double_real (pop determined by opcode).
    pub fn fcomp_double_real(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fcom_double_real(instr)
    }

    // =======================================================================
    // FICOM / FICOMP word integer (i16 memory operand)
    // =======================================================================

    /// FICOM word integer — Compare ST(0) with a 16-bit signed integer from memory.
    /// Also used for FICOMP (pops after comparison).
    pub fn ficom_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let pop_stack = instr.get_ia_opcode() == Opcode::FicompWordInteger;

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_word(seg, eaddr)? as i16;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            i32_to_extf80(load_reg as i32),
            false,
            &mut status,
        );
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FICOMP word integer — Compare ST(0) with i16 from memory and pop.
    /// Dispatches to ficom_word_integer (pop determined by opcode).
    pub fn ficomp_word_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.ficom_word_integer(instr)
    }

    // =======================================================================
    // FICOM / FICOMP dword integer (i32 memory operand)
    // =======================================================================

    /// FICOM dword integer — Compare ST(0) with a 32-bit signed integer from memory.
    /// Also used for FICOMP (pops after comparison).
    pub fn ficom_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();

        let pop_stack = instr.get_ia_opcode() == Opcode::FicompDwordInteger;

        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let load_reg = self.read_virtual_dword(seg, eaddr)? as i32;

        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            i32_to_extf80(load_reg),
            false,
            &mut status,
        );
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FICOMP dword integer — Compare ST(0) with i32 from memory and pop.
    /// Dispatches to ficom_dword_integer (pop determined by opcode).
    pub fn ficomp_dword_integer(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.ficom_dword_integer(instr)
    }

    // =======================================================================
    // FCOMPP / FUCOMPP  (compare and double-pop)
    // =======================================================================

    /// FCOMPP — Compare ST(0) with ST(1) and pop both.
    /// Also used for FUCOMPP (quiet comparison — only SNaN raises invalid).
    pub fn fcompp(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(1) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);

            if self.the_i387.is_ia_masked() {
                self.the_i387.fpu_pop();
                self.the_i387.fpu_pop();
            }
            return Ok(());
        }

        let quiet = instr.get_ia_opcode() == Opcode::Fucompp;

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            self.read_fpu_reg(1),
            quiet,
            &mut status,
        );
        self.setcc(status_word_flags_fpu_compare(rc));

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            self.the_i387.fpu_pop();
            self.the_i387.fpu_pop();
        }

        Ok(())
    }

    // =======================================================================
    // FCOMI ST(0),ST(j) / FCOMIP ST(0),ST(j)
    // =======================================================================

    /// FCOMI ST(0),ST(j) — Compare ST(0) with ST(src) and set EFLAGS (CF, ZF, PF).
    /// Signaling comparison: both SNaN and QNaN raise invalid.
    /// Also used for FCOMIP (pops after comparison).
    pub fn fcomi_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = instr.get_ia_opcode() == Opcode::FcomipSt0Stj;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            // Set EFLAGS to unordered: ZF=1, PF=1, CF=1
            self.write_eflags_fpu_compare(RELATION_UNORDERED);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            self.read_fpu_reg(instr.src() as i32),
            false, // signaling (not quiet)
            &mut status,
        );
        self.write_eflags_fpu_compare(rc);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FCOMIP ST(0),ST(j) — Compare ST(0) with ST(src), set EFLAGS, and pop.
    /// Dispatches to fcomi_st0_stj (pop determined by opcode).
    pub fn fcomip_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fcomi_st0_stj(instr)
    }

    // =======================================================================
    // FUCOMI ST(0),ST(j) / FUCOMIP ST(0),ST(j)
    // =======================================================================

    /// FUCOMI ST(0),ST(j) — Unordered compare ST(0) with ST(src), set EFLAGS.
    /// Quiet comparison: only SNaN raises invalid (QNaN does not).
    /// Also used for FUCOMIP (pops after comparison).
    pub fn fucomi_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let pop_stack = instr.get_ia_opcode() == Opcode::FucomipSt0Stj;

        self.clear_c1();

        if self.is_tag_empty(0) || self.is_tag_empty(instr.src() as i32) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            // Set EFLAGS to unordered: ZF=1, PF=1, CF=1
            self.write_eflags_fpu_compare(RELATION_UNORDERED);

            if self.the_i387.is_ia_masked() {
                if pop_stack {
                    self.the_i387.fpu_pop();
                }
            }
            return Ok(());
        }

        let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

        let rc = extf80_compare(
            self.read_fpu_reg(0),
            self.read_fpu_reg(instr.src() as i32),
            true, // quiet (unordered)
            &mut status,
        );
        self.write_eflags_fpu_compare(rc);

        if self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false) == 0 {
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }

        Ok(())
    }

    /// FUCOMIP ST(0),ST(j) — Unordered compare, set EFLAGS, and pop.
    /// Dispatches to fucomi_st0_stj (pop determined by opcode).
    pub fn fucomip_st0_stj(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fucomi_st0_stj(instr)
    }

    // =======================================================================
    // FTST  (compare ST(0) with +0.0)
    // =======================================================================

    /// FTST — Compare ST(0) with positive zero and set condition codes.
    pub fn ftst(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        self.clear_c1();

        if self.is_tag_empty(0) {
            self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
            self.setcc(FPU_SW_C0 | FPU_SW_C2 | FPU_SW_C3);
        } else {
            let const_z = pack_floatx80(false, 0, 0);

            let mut status = i387cw_to_softfloat_status_word(self.the_i387.get_control_word());

            let rc = extf80_compare(self.read_fpu_reg(0), const_z, false, &mut status);
            self.setcc(status_word_flags_fpu_compare(rc));
            self.fpu_exception(instr, status.softfloat_exceptionFlags as u32, false);
        }

        Ok(())
    }

    // =======================================================================
    // FXAM  (examine ST(0))
    // =======================================================================

    /// FXAM — Examine ST(0) and set C3/C2/C1/C0 based on the class of value.
    ///
    /// C1 is set to the sign of the value in ST(0), regardless of whether the
    /// register is empty or full.
    pub fn fxam(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions();
        self.fpu_update_last_instruction(instr);

        let reg = self.read_fpu_reg(0);
        let sign = extf80_sign(reg);

        if self.is_tag_empty(0) {
            // Empty: C3=1, C1=1, C0=1
            self.setcc(FPU_SW_C3 | FPU_SW_C1 | FPU_SW_C0);
        } else {
            let a_class = extf80_class(reg);

            match a_class {
                SoftFloatClass::Zero => {
                    // Zero: C3=1, C1=1
                    self.setcc(FPU_SW_C3 | FPU_SW_C1);
                }
                SoftFloatClass::SNaN | SoftFloatClass::QNaN => {
                    // NaN: unsupported reported as just C1, otherwise C1|C0
                    if extf80_is_unsupported(reg) {
                        self.setcc(FPU_SW_C1);
                    } else {
                        self.setcc(FPU_SW_C1 | FPU_SW_C0);
                    }
                }
                SoftFloatClass::NegativeInf | SoftFloatClass::PositiveInf => {
                    // Infinity: C2=1, C1=1, C0=1
                    self.setcc(FPU_SW_C2 | FPU_SW_C1 | FPU_SW_C0);
                }
                SoftFloatClass::Denormal => {
                    // Denormal: C3=1, C2=1, C1=1
                    self.setcc(FPU_SW_C3 | FPU_SW_C2 | FPU_SW_C1);
                }
                SoftFloatClass::Normalized => {
                    // Normal: C2=1, C1=1
                    self.setcc(FPU_SW_C2 | FPU_SW_C1);
                }
            }
        }

        // C1 is set to the sign of the value in ST(0), regardless of
        // whether the register is empty or full.  All the setcc() calls
        // above set C1=1, so we only need to clear it for positive values.
        if !sign {
            self.clear_c1();
        }

        Ok(())
    }
}
