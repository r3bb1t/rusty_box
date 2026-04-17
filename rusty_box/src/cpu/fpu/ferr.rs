#![allow(dead_code)]
//! FPU exception infrastructure for x86 CPU emulation.
//!
//! Ported from Bochs `cpu/fpu/ferr.cc`, parts of `cpu/fpu/fpu.cc`,
//! and `cpu/fpu/fpu_arith.cc`.
//!
//! Implements:
//! - FPU_exception: main exception reporting (sets status word bits, handles masking)
//! - FPU_stack_overflow / FPU_stack_underflow: push/pop with default-NaN on masked IA
//! - FPU_check_pending_exceptions: raises #MF when summary bit is set and CR0.NE=1
//! - FPU_update_last_instruction: saves CS:RIP and DS:addr for FXSAVE/FSTENV
//! - FPU_tagof: classify an 80-bit register into Valid/Zero/Special/Empty
//! - i387cw_to_softfloat_status_word: convert i387 control word to SoftFloat status
//! - Helper accessors: is_tag_empty, read/write FPU registers, setcc, clear_c1
//! - write_eflags_fpu_compare: for FCOMI/FUCOMI (sets EFLAGS ZF/PF/CF)

use super::super::cpu::BxCpuC;
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::{BxSegregs, Instruction};
use super::super::i387::*;
use super::super::softfloat3e::softfloat::*;
use super::super::softfloat3e::specialize::*;

use super::super::cpu::Exception;
use super::super::softfloat3e::softfloat_types::floatx80;
use crate::cpu::eflags::EFlags;

// ---------------------------------------------------------------------------
// Free function: convert i387 control word → SoftFloat status
// ---------------------------------------------------------------------------

/// Convert an i387 control word to a `SoftFloatStatus` structure suitable for
/// passing to the SoftFloat 3e arithmetic routines.
///
/// Ported from Bochs `i387cw_to_softfloat_status_word` in `fpu_arith.cc`.
pub fn i387cw_to_softfloat_status_word(control_word: u16) -> SoftFloatStatus {
    let precision = control_word & FPU_CW_PC;
    let rounding_precision = match precision {
        FPU_PR_32_BITS => 32,
        FPU_PR_64_BITS => 64,
        FPU_PR_80_BITS => 80,
        _ => 80,
    };

    SoftFloatStatus {
        softfloat_roundingMode: ((control_word & FPU_CW_RC) >> 10) as u8,
        softfloat_exceptionFlags: 0,
        softfloat_exceptionMasks: (control_word & FPU_CW_EXCEPTIONS_MASK) as i32,
        softfloat_suppressException: 0,
        softfloat_denormals_are_zeros: false,
        softfloat_flush_underflow_to_zero: false,
        extF80_roundingPrecision: rounding_precision,
    }
}

// ---------------------------------------------------------------------------
// Methods on BxCpuC
// ---------------------------------------------------------------------------

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // -----------------------------------------------------------------------
    // FPU_check_pending_exceptions  (from fpu.cc)
    // -----------------------------------------------------------------------

    /// Check if unmasked FPU exceptions are pending and, if CR0.NE=1,
    /// raise #MF (exception 16).
    ///
    /// In MSDOS-compatibility mode (NE=0) we just log a warning.
    pub fn fpu_check_pending_exceptions(&mut self) -> super::super::Result<()> {
        if (self.the_i387.get_partial_status() & FPU_SW_SUMMARY) != 0 {
            // CR0.NE is bit 5
            if self.cr0.ne() {
                // Native FPU error reporting — raise #MF
                return self.exception(Exception::Mf, 0u16);
            } else {
                tracing::info!("math_abort: MSDOS compatibility FPU exception");
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // FPU_update_last_instruction  (from fpu.cc)
    // -----------------------------------------------------------------------

    /// Save the CS:RIP of the faulting FPU instruction, and if the instruction
    /// has a memory operand, save the DS:addr as well.  These are used by
    /// FXSAVE, FSTENV, etc.
    pub fn fpu_update_last_instruction(&mut self, instr: &Instruction) {
        self.the_i387.fcs = self.sregs[BxSegregs::Cs as usize].selector.value;
        self.the_i387.fip = self.prev_rip;

        if !instr.mod_c0() {
            let seg_idx = instr.seg() as usize;
            self.the_i387.fds = self.sregs[seg_idx].selector.value;
            // Resolve the effective address that the instruction references.
            let eaddr = self.resolve_addr(instr);
            self.the_i387.fdp = eaddr;
        }
    }

    // -----------------------------------------------------------------------
    // FPU_exception  (from ferr.cc)
    // -----------------------------------------------------------------------

    /// Main FPU exception handler.  Updates the status word according to the
    /// exception bits, respects the control-word mask, and returns a bitmask
    /// of *unmasked* exception bits.
    ///
    /// `exception` uses the FPU_SW_* / FPU_EX_* bit definitions (which are
    /// identical in value to the softfloat FLAG_* constants).
    ///
    /// `is_store` affects whether unmasked overflow/underflow are reported
    /// (only significant for store operations).
    pub fn fpu_exception(&mut self, _instr: &Instruction, exception: u32, is_store: bool) -> u32 {
        let exception = exception & (FPU_SW_EXCEPTIONS_MASK as u32);
        let status = self.the_i387.swd as u32;
        let cw = self.the_i387.cwd as u32;
        let mut unmasked = exception & !cw & (FPU_CW_EXCEPTIONS_MASK as u32);

        // For Invalid / Zero-Div, only those bits can be unmasked
        if exception & ((FPU_SW_INVALID as u32) | (FPU_SW_ZERO_DIV as u32)) != 0 {
            unmasked &= (FPU_SW_INVALID as u32) | (FPU_SW_ZERO_DIV as u32);
        }

        if unmasked != 0 {
            self.the_i387.swd |= FPU_SW_SUMMARY | FPU_SW_BACKWARD;
        }

        // --- Invalid ---
        if exception & (FPU_SW_INVALID as u32) != 0 {
            self.the_i387.swd |= exception as u16;
            if exception & (FPU_SW_STACK_FAULT as u32) != 0
                && exception & (FPU_SW_C1 as u32) == 0 {
                    self.the_i387.swd &= !(FPU_SW_C1);
                }
            return unmasked;
        }

        // --- Zero divide ---
        if exception & (FPU_SW_ZERO_DIV as u32) != 0 {
            self.the_i387.swd |= FPU_SW_ZERO_DIV;
            return unmasked;
        }

        // --- Denormal ---
        if exception & (FPU_SW_DENORMAL_OP as u32) != 0 {
            self.the_i387.swd |= FPU_SW_DENORMAL_OP;
            if unmasked & (FPU_SW_DENORMAL_OP as u32) != 0 {
                return unmasked & (FPU_SW_DENORMAL_OP as u32);
            }
        }

        // --- Remaining exceptions (Precision, Overflow, Underflow) ---
        self.the_i387.swd |= exception as u16;

        if exception & (FPU_SW_PRECISION as u32) != 0
            && exception & (FPU_SW_C1 as u32) == 0 {
                self.the_i387.swd &= !(FPU_SW_C1);
            }

        // For overflow/underflow, masking depends on whether this is a store.
        let mut unmasked = unmasked & !(FPU_SW_PRECISION as u32);

        if unmasked & ((FPU_SW_UNDERFLOW as u32) | (FPU_SW_OVERFLOW as u32)) != 0 {
            if !is_store {
                unmasked &= !((FPU_SW_UNDERFLOW as u32) | (FPU_SW_OVERFLOW as u32));
            } else {
                self.the_i387.swd &= !(FPU_SW_C1);
                if (status & (FPU_SW_PRECISION as u32)) == 0 {
                    self.the_i387.swd &= !(FPU_SW_PRECISION);
                }
            }
        }

        unmasked
    }

    // -----------------------------------------------------------------------
    // FPU_stack_overflow  (from ferr.cc)
    // -----------------------------------------------------------------------

    /// Handle FPU stack overflow: if the Invalid-Arithmetic exception is
    /// masked, push a default-NaN onto the stack.  Then report the exception.
    pub fn fpu_stack_overflow(&mut self, instr: &Instruction) {
        if self.the_i387.is_ia_masked() {
            self.the_i387.fpu_push();
            self.write_fpu_reg(FLOATX80_DEFAULT_NAN, 0);
        }
        self.fpu_exception(instr, FPU_EX_STACK_OVERFLOW as u32, false);
    }

    // -----------------------------------------------------------------------
    // FPU_stack_underflow  (from ferr.cc)
    // -----------------------------------------------------------------------

    /// Handle FPU stack underflow: if the IA exception is masked, write a
    /// default-NaN to `st(stnr)` and optionally pop.  Then report the exception.
    pub fn fpu_stack_underflow(&mut self, instr: &Instruction, stnr: i32, pop_stack: bool) {
        if self.the_i387.is_ia_masked() {
            self.write_fpu_reg(FLOATX80_DEFAULT_NAN, stnr);
            if pop_stack {
                self.the_i387.fpu_pop();
            }
        }
        self.fpu_exception(instr, FPU_EX_STACK_UNDERFLOW as u32, false);
    }

    // -----------------------------------------------------------------------
    // FPU_tagof  (from fpu.cc)
    // -----------------------------------------------------------------------

    /// Classify a floatx80 register value into a tag:
    /// `FPU_TAG_VALID`, `FPU_TAG_ZERO`, or `FPU_TAG_SPECIAL`.
    ///
    /// Note: this does NOT return `FPU_TAG_EMPTY` — that is determined by the
    /// tag word, not the register contents.
    pub fn fpu_tagof(reg: &floatx80) -> i32 {
        let exp = extf80_exp(*reg);
        if exp == 0 {
            if extf80_fraction(*reg) == 0 {
                return FPU_TAG_ZERO as i32;
            }
            return FPU_TAG_SPECIAL as i32;
        }
        if exp == 0x7FFF {
            return FPU_TAG_SPECIAL as i32;
        }
        // Check integer bit (bit 63 of significand) — must be set for a
        // normalised number.
        if (reg.signif & 0x8000000000000000) == 0 {
            return FPU_TAG_SPECIAL as i32;
        }
        FPU_TAG_VALID as i32
    }

    // -----------------------------------------------------------------------
    // Helper: is_tag_empty
    // -----------------------------------------------------------------------

    /// Return `true` if register `st(stnr)` is tagged as empty.
    #[inline]
    pub fn is_tag_empty(&self, stnr: i32) -> bool {
        self.the_i387.fpu_gettagi(stnr) == FPU_TAG_EMPTY as i32
    }

    // -----------------------------------------------------------------------
    // Helper: read_fpu_reg
    // -----------------------------------------------------------------------

    /// Read the floatx80 value from `st(stnr)`.
    #[inline]
    pub fn read_fpu_reg(&self, stnr: i32) -> floatx80 {
        self.the_i387.fpu_read_regi(stnr)
    }

    // -----------------------------------------------------------------------
    // Helper: write_fpu_reg
    // -----------------------------------------------------------------------

    /// Write a floatx80 value to `st(stnr)` and mark its tag as Valid.
    #[inline]
    pub fn write_fpu_reg(&mut self, reg: floatx80, stnr: i32) {
        self.the_i387.fpu_save_regi(reg, stnr);
    }

    // -----------------------------------------------------------------------
    // Helper: write_fpu_reg_with_tag
    // -----------------------------------------------------------------------

    /// Write a floatx80 value to `st(stnr)` with an explicit tag.
    #[inline]
    pub fn write_fpu_reg_with_tag(&mut self, reg: floatx80, tag: i32, stnr: i32) {
        self.the_i387.fpu_save_regi_with_tag(reg, tag, stnr);
    }

    // -----------------------------------------------------------------------
    // Helper: setcc  —  set condition-code bits in the FPU status word
    // -----------------------------------------------------------------------

    /// Replace the C0/C1/C2/C3 condition-code bits in the FPU status word.
    /// `cc` should contain the desired bits in the FPU_SW_CC mask positions.
    #[inline]
    pub fn setcc(&mut self, cc: u16) {
        self.the_i387.swd = (self.the_i387.swd & !FPU_SW_CC) | (cc & FPU_SW_CC);
    }

    // -----------------------------------------------------------------------
    // Helper: clear_c1  —  clear the C1 condition-code bit
    // -----------------------------------------------------------------------

    /// Clear the C1 bit in the FPU status word.  Many FPU operations clear C1
    /// to indicate that the result was NOT rounded up.
    #[inline]
    pub fn clear_c1(&mut self) {
        self.the_i387.swd &= !FPU_SW_C1;
    }

    // -----------------------------------------------------------------------
    // write_eflags_fpu_compare  —  for FCOMI / FUCOMI
    // -----------------------------------------------------------------------

    /// Set EFLAGS ZF, PF, CF from a softfloat comparison result for
    /// FCOMI / FUCOMI / FCOMIP / FUCOMIP instructions.
    ///
    /// Mapping (Intel SDM Vol 1, Table 8-5):
    /// ```text
    ///   Relation          ZF  PF  CF
    ///   ST(0) > src        0   0   0
    ///   ST(0) < src        0   0   1
    ///   ST(0) = src        1   0   0
    ///   Unordered          1   1   1
    /// ```
    pub fn write_eflags_fpu_compare(&mut self, float_relation: i32) {
        // Bochs clearEFlagsOSZAPC(): clear OF, SF, ZF, AF, PF, CF
        self.eflags
            .remove(EFlags::CF | EFlags::PF | EFlags::AF | EFlags::ZF | EFlags::SF | EFlags::OF);

        match float_relation {
            RELATION_LESS => {
                // ST(0) < src: CF=1, PF=0, ZF=0
                self.eflags.insert(EFlags::CF);
            }
            RELATION_EQUAL => {
                // ST(0) == src: CF=0, PF=0, ZF=1
                self.eflags.insert(EFlags::ZF);
            }
            RELATION_GREATER => {
                // ST(0) > src: CF=0, PF=0, ZF=0
                // (all cleared above)
            }
            _ => {
                // Unordered (NaN): CF=1, PF=1, ZF=1
                self.eflags.insert(EFlags::CF | EFlags::PF | EFlags::ZF);
            }
        }
    }
}
