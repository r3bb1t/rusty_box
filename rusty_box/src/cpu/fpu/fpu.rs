#![allow(dead_code)]
//! FPU core instructions: FNINIT, FNCLEX, FNOP, FPLEGACY, FLDCW, FNSTCW, FNSTSW, FNSTSW_AX,
//! FLDENV, FNSTENV, FRSTOR, FNSAVE
//! Ported from Bochs cpu/fpu/fpu.cc

use super::super::cpu::{BxCpuC, CpuMode};
use super::super::cpuid::BxCpuIdTrait;
use super::super::decoder::{BxSegregs, Instruction};
use super::super::i387::*;
use super::super::softfloat3e::softfloat_types::floatx80;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// FNINIT — Initialize FPU state
    pub fn fninit(&mut self, _instr: &Instruction) -> super::super::Result<()> {
        self.the_i387.init();
        Ok(())
    }

    /// FNCLEX — Clear FPU exception flags
    pub fn fnclex(&mut self, _instr: &Instruction) -> super::super::Result<()> {
        self.the_i387.swd &= !(FPU_SW_BACKWARD
            | FPU_SW_SUMMARY
            | FPU_SW_STACK_FAULT
            | FPU_SW_PRECISION
            | FPU_SW_UNDERFLOW
            | FPU_SW_OVERFLOW
            | FPU_SW_ZERO_DIV
            | FPU_SW_DENORMAL_OP
            | FPU_SW_INVALID);
        Ok(())
    }

    /// FNOP — No operation
    pub fn fnop(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_update_last_instruction(instr);
        Ok(())
    }

    /// FPLEGACY — Legacy FPU prefix (no operation)
    pub fn fplegacy(&mut self, _instr: &Instruction) -> super::super::Result<()> {
        Ok(())
    }

    /// FLDCW — Load control word from memory
    pub fn fldcw(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let cwd = self.v_read_word(seg, eaddr)?;
        self.the_i387.cwd = (cwd & !FPU_CW_RESERVED_BITS) | 0x0040;

        // Check for unmasked exceptions
        if (self.the_i387.swd & !self.the_i387.cwd & FPU_CW_EXCEPTIONS_MASK) != 0 {
            self.the_i387.swd |= FPU_SW_SUMMARY | FPU_SW_BACKWARD;
        } else {
            self.the_i387.swd &= !(FPU_SW_SUMMARY | FPU_SW_BACKWARD);
        }

        Ok(())
    }

    /// FNSTCW — Store control word to memory
    pub fn fnstcw(&mut self, instr: &Instruction) -> super::super::Result<()> {
        let cwd = self.the_i387.get_control_word();

        if !instr.mod_c0() {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_write_word(seg, eaddr, cwd)?;
        }

        Ok(())
    }

    /// FNSTSW — Store status word to memory
    pub fn fnstsw(&mut self, instr: &Instruction) -> super::super::Result<()> {
        let swd = self.the_i387.get_status_word();

        if !instr.mod_c0() {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_write_word(seg, eaddr, swd)?;
        }

        Ok(())
    }

    /// FNSTSW AX — Store status word to AX register
    pub fn fnstsw_ax(&mut self, _instr: &Instruction) -> super::super::Result<()> {
        let swd = self.the_i387.get_status_word();
        self.set_rax((self.rax() & !0xFFFF) | swd as u64);
        Ok(())
    }

    /// FNSTENV — Store FPU environment (14/28 bytes)
    pub fn fnstenv(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_save_environment(instr)?;
        // Mask all floating point exceptions
        self.the_i387.cwd |= FPU_CW_EXCEPTIONS_MASK;
        // Clear B and ES bits
        self.the_i387.swd &= !(FPU_SW_BACKWARD | FPU_SW_SUMMARY);
        Ok(())
    }

    /// FLDENV — Load FPU environment (14/28 bytes)
    pub fn fldenv(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;
        self.fpu_load_environment(instr)?;

        // Update tags for non-empty registers based on actual content
        for n in 0..8 {
            if !self.is_tag_empty(n) {
                let reg = self.read_fpu_reg(n);
                let tag = Self::fpu_tagof(&reg);
                self.the_i387.fpu_settagi(tag, n);
            }
        }

        Ok(())
    }

    /// FNSAVE — Save full FPU state (94/108 bytes)
    pub fn fnsave(&mut self, instr: &Instruction) -> super::super::Result<()> {
        let offset = self.fpu_save_environment(instr)?;
        let seg = BxSegregs::from(instr.seg());
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() != 0 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        };

        // Save all 8 registers in stack order
        for n in 0..8i32 {
            let stn = self.read_fpu_reg(n);
            let reg_offset = (offset.wrapping_add((n as u64) * 10)) & asize_mask;
            // Write 10-byte extended precision: 8-byte significand + 2-byte sign/exponent
            let lo = stn.signif as u32;
            let hi = (stn.signif >> 32) as u32;
            self.v_write_dword(seg, reg_offset, lo)?;
            self.v_write_dword(seg, reg_offset.wrapping_add(4) & asize_mask, hi)?;
            self.v_write_word(seg, reg_offset.wrapping_add(8) & asize_mask, stn.sign_exp)?;
        }

        // FNINIT after save
        self.the_i387.init();

        Ok(())
    }

    /// FRSTOR — Restore full FPU state (94/108 bytes)
    pub fn frstor(&mut self, instr: &Instruction) -> super::super::Result<()> {
        self.fpu_check_pending_exceptions()?;

        let offset = self.fpu_load_environment(instr)?;
        let seg = BxSegregs::from(instr.seg());
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() != 0 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        };

        // Read all 8 registers in stack order
        for n in 0..8i32 {
            let reg_offset = (offset.wrapping_add((n as u64) * 10)) & asize_mask;
            let lo = self.v_read_dword(seg, reg_offset)? as u64;
            let hi = self.v_read_dword(seg, reg_offset.wrapping_add(4) & asize_mask)? as u64;
            let signif = lo | (hi << 32);
            let sign_exp = self.v_read_word(seg, reg_offset.wrapping_add(8) & asize_mask)?;
            let tmp = floatx80 { signif, sign_exp };

            let tag = if self.is_tag_empty(n) {
                FPU_TAG_EMPTY as i32
            } else {
                Self::fpu_tagof(&tmp)
            };
            self.write_fpu_reg_with_tag(tmp, tag, n);
        }

        Ok(())
    }

    // --- Environment save/load helpers ---

    /// Check if in protected mode (not real mode and not V8086)
    #[inline]
    fn is_protected_mode(&self) -> bool {
        self.cpu_mode == CpuMode::Ia32Protected
            || self.cpu_mode == CpuMode::LongCompat
            || self.cpu_mode == CpuMode::Long64
    }

    fn fpu_save_environment(&mut self, instr: &Instruction) -> super::super::Result<u64> {
        // Update tags for non-empty registers
        for n in 0..8 {
            if !self.is_tag_empty(n) {
                let reg = self.read_fpu_reg(n);
                let tag = Self::fpu_tagof(&reg);
                self.the_i387.fpu_settagi(tag, n);
            }
        }

        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() != 0 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        };

        let offset: u64;

        if self.is_protected_mode() {
            if instr.os32_l() != 0 {
                // Protected mode - 32 bit
                let tmp = 0xFFFF0000u32 | self.the_i387.get_control_word() as u32;
                self.v_write_dword(seg, eaddr, tmp)?;
                let tmp = 0xFFFF0000u32 | self.the_i387.get_status_word() as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x04) & asize_mask, tmp)?;
                let tmp = 0xFFFF0000u32 | self.the_i387.get_tag_word() as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x08) & asize_mask, tmp)?;
                let tmp = self.the_i387.fip as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x0c) & asize_mask, tmp)?;
                let tmp = (self.the_i387.fcs as u32) | ((self.the_i387.foo as u32) << 16);
                self.v_write_dword(seg, eaddr.wrapping_add(0x10) & asize_mask, tmp)?;
                let tmp = self.the_i387.fdp as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x14) & asize_mask, tmp)?;
                let tmp = 0xFFFF0000u32 | self.the_i387.fds as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x18) & asize_mask, tmp)?;
                offset = 0x1c;
            } else {
                // Protected mode - 16 bit
                self.v_write_word(seg, eaddr, self.the_i387.get_control_word())?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x02) & asize_mask,
                    self.the_i387.get_status_word(),
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x04) & asize_mask,
                    self.the_i387.get_tag_word(),
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x06) & asize_mask,
                    (self.the_i387.fip & 0xFFFF) as u16,
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x08) & asize_mask,
                    self.the_i387.fcs,
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x0a) & asize_mask,
                    (self.the_i387.fdp & 0xFFFF) as u16,
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x0c) & asize_mask,
                    self.the_i387.fds,
                )?;
                offset = 0x0e;
            }
        } else {
            // Real or V86 mode
            let fp_ip = ((self.the_i387.fcs as u32) << 4).wrapping_add(self.the_i387.fip as u32);
            let fp_dp = ((self.the_i387.fds as u32) << 4).wrapping_add(self.the_i387.fdp as u32);

            if instr.os32_l() != 0 {
                let tmp = 0xFFFF0000u32 | self.the_i387.get_control_word() as u32;
                self.v_write_dword(seg, eaddr, tmp)?;
                let tmp = 0xFFFF0000u32 | self.the_i387.get_status_word() as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x04) & asize_mask, tmp)?;
                let tmp = 0xFFFF0000u32 | self.the_i387.get_tag_word() as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x08) & asize_mask, tmp)?;
                let tmp = 0xFFFF0000u32 | (fp_ip & 0xFFFF);
                self.v_write_dword(seg, eaddr.wrapping_add(0x0c) & asize_mask, tmp)?;
                let tmp = ((fp_ip & 0xFFFF0000) >> 4) | self.the_i387.foo as u32;
                self.v_write_dword(seg, eaddr.wrapping_add(0x10) & asize_mask, tmp)?;
                let tmp = 0xFFFF0000u32 | (fp_dp & 0xFFFF);
                self.v_write_dword(seg, eaddr.wrapping_add(0x14) & asize_mask, tmp)?;
                let tmp = (fp_dp & 0xFFFF0000) >> 4;
                self.v_write_dword(seg, eaddr.wrapping_add(0x18) & asize_mask, tmp)?;
                offset = 0x1c;
            } else {
                // Real mode - 16 bit
                self.v_write_word(seg, eaddr, self.the_i387.get_control_word())?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x02) & asize_mask,
                    self.the_i387.get_status_word(),
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x04) & asize_mask,
                    self.the_i387.get_tag_word(),
                )?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x06) & asize_mask,
                    (fp_ip & 0xFFFF) as u16,
                )?;
                let tmp = ((fp_ip & 0xF0000) >> 4) as u16 | self.the_i387.foo;
                self.v_write_word(seg, eaddr.wrapping_add(0x08) & asize_mask, tmp)?;
                self.v_write_word(
                    seg,
                    eaddr.wrapping_add(0x0a) & asize_mask,
                    (fp_dp & 0xFFFF) as u16,
                )?;
                let tmp = ((fp_dp & 0xF0000) >> 4) as u16;
                self.v_write_word(seg, eaddr.wrapping_add(0x0c) & asize_mask, tmp)?;
                offset = 0x0e;
            }
        }

        Ok(eaddr.wrapping_add(offset) & asize_mask)
    }

    fn fpu_load_environment(&mut self, instr: &Instruction) -> super::super::Result<u64> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let asize_mask: u64 = if self.long64_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else if instr.as32_l() != 0 {
            0xFFFF_FFFF
        } else {
            0xFFFF
        };

        let offset: u64;

        if self.is_protected_mode() {
            if instr.os32_l() != 0 {
                // Protected mode - 32 bit
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x18) & asize_mask)?;
                self.the_i387.fds = (tmp & 0xFFFF) as u16;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x14) & asize_mask)?;
                self.the_i387.fdp = tmp as u64;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x10) & asize_mask)?;
                self.the_i387.fcs = (tmp & 0xFFFF) as u16;
                self.the_i387.foo = ((tmp >> 16) & 0x07FF) as u16;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x0c) & asize_mask)?;
                self.the_i387.fip = tmp as u64;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x08) & asize_mask)?;
                self.the_i387.twd = (tmp & 0xFFFF) as u16;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x04) & asize_mask)?;
                self.the_i387.swd = (tmp & 0xFFFF) as u16;
                self.the_i387.tos = ((tmp >> 11) & 0x7) as u8;
                let tmp = self.v_read_dword(seg, eaddr)?;
                self.the_i387.cwd = (tmp & 0xFFFF) as u16;
                offset = 0x1c;
            } else {
                // Protected mode - 16 bit
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x0c) & asize_mask)?;
                self.the_i387.fds = tmp;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x0a) & asize_mask)?;
                self.the_i387.fdp = tmp as u64;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x08) & asize_mask)?;
                self.the_i387.fcs = tmp;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x06) & asize_mask)?;
                self.the_i387.fip = tmp as u64;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x04) & asize_mask)?;
                self.the_i387.twd = tmp;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x02) & asize_mask)?;
                self.the_i387.swd = tmp;
                self.the_i387.tos = ((tmp >> 11) & 0x7) as u8;
                let tmp = self.v_read_word(seg, eaddr)?;
                self.the_i387.cwd = tmp;
                self.the_i387.foo = 0;
                offset = 0x0e;
            }
        } else {
            // Real or V86 mode
            if instr.os32_l() != 0 {
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x18) & asize_mask)?;
                let fp_dp_hi = (tmp & 0x0FFFF000) << 4;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x14) & asize_mask)?;
                let fp_dp = fp_dp_hi | (tmp & 0xFFFF);
                self.the_i387.fdp = fp_dp as u64;
                self.the_i387.fds = 0;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x10) & asize_mask)?;
                self.the_i387.foo = (tmp & 0x07FF) as u16;
                let fp_ip_hi = (tmp & 0x0FFFF000) << 4;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x0c) & asize_mask)?;
                let fp_ip = fp_ip_hi | (tmp & 0xFFFF);
                self.the_i387.fip = fp_ip as u64;
                self.the_i387.fcs = 0;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x08) & asize_mask)?;
                self.the_i387.twd = (tmp & 0xFFFF) as u16;
                let tmp = self.v_read_dword(seg, eaddr.wrapping_add(0x04) & asize_mask)?;
                self.the_i387.swd = (tmp & 0xFFFF) as u16;
                self.the_i387.tos = ((tmp >> 11) & 0x7) as u8;
                let tmp = self.v_read_dword(seg, eaddr)?;
                self.the_i387.cwd = (tmp & 0xFFFF) as u16;
                offset = 0x1c;
            } else {
                // Real mode - 16 bit
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x0c) & asize_mask)?;
                let fp_dp_hi = ((tmp & 0xF000) as u32) << 4;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x0a) & asize_mask)?;
                self.the_i387.fdp = (fp_dp_hi | tmp as u32) as u64;
                self.the_i387.fds = 0;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x08) & asize_mask)?;
                self.the_i387.foo = tmp & 0x07FF;
                let fp_ip_hi = ((tmp & 0xF000) as u32) << 4;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x06) & asize_mask)?;
                self.the_i387.fip = (fp_ip_hi | tmp as u32) as u64;
                self.the_i387.fcs = 0;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x04) & asize_mask)?;
                self.the_i387.twd = tmp;
                let tmp = self.v_read_word(seg, eaddr.wrapping_add(0x02) & asize_mask)?;
                self.the_i387.swd = tmp;
                self.the_i387.tos = ((tmp >> 11) & 0x7) as u8;
                let tmp = self.v_read_word(seg, eaddr)?;
                self.the_i387.cwd = tmp;
                offset = 0x0e;
            }
        }

        // Always set bit 6 as '1'
        self.the_i387.cwd = (self.the_i387.cwd & !FPU_CW_RESERVED_BITS) | 0x0040;

        // Check for unmasked exceptions
        if (self.the_i387.swd & !self.the_i387.cwd & FPU_CW_EXCEPTIONS_MASK) != 0 {
            self.the_i387.swd |= FPU_SW_SUMMARY | FPU_SW_BACKWARD;
        } else {
            self.the_i387.swd &= !(FPU_SW_SUMMARY | FPU_SW_BACKWARD);
        }

        Ok(eaddr.wrapping_add(offset) & asize_mask)
    }
}
