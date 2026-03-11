//! 64-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical64.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Note: CMP, INC, DEC register/memory forms are implemented in arith64.rs
//! to avoid duplication. This file contains XOR, OR, AND, NOT, TEST,
//! the CMP_EqGqR register-form (not in arith64), CMP_RAXId, and unified
//! dispatchers that call into both this file and arith64.rs.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 64-bit logical operations (AND, OR, XOR, TEST).
    /// Clears OF and CF, sets SF/ZF/PF from result, AF is undefined (cleared).
    /// Matches Bochs SET_FLAGS_OSZAPC_LOGIC_64 macro.
    pub(super) fn set_flags_oszapc_logic_64(&mut self, result: u64) {
        let sf = (result & 0x8000_0000_0000_0000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        self.eflags.remove(EFlags::LOGIC_MASK);

        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
    }

    // =========================================================================
    // XOR instructions
    // =========================================================================

    /// XOR_EqGqM: XOR r/m64, r64 (memory form)
    /// Bochs: XOR_EqGqM — read-modify-write memory, src from register
    pub(super) fn xor_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);
        let result = op1_64 ^ op2_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// XOR_GqEqR: XOR r64, r64 (register form, load-direction)
    /// Bochs: XOR_GqEqR
    pub(super) fn xor_gq_eq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1_64 = self.get_gpr64(dst);
        let op2_64 = self.get_gpr64(src);
        let result = op1_64 ^ op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// XOR_GqEqM: XOR r64, r/m64 (memory form, load-direction)
    /// Bochs: XOR_GqEqM — register destination, memory source
    pub(super) fn xor_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_64 = self.read_linear_qword(seg, laddr)?;
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = op1_64 ^ op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// XOR_EqIdM: XOR r/m64, imm32 (sign-extended) (memory form)
    /// Bochs: XOR_EqIdM — imm32 sign-extended to 64 bits
    pub(super) fn xor_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = op1_64 ^ op2_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// XOR_EqIdR: XOR r64, imm32 (sign-extended) (register form)
    /// Bochs: XOR_EqIdR — imm32 sign-extended to 64 bits
    pub(super) fn xor_eq_id_r(&mut self, instr: &Instruction) {
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = op1_64 ^ op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
    }

    // =========================================================================
    // OR instructions
    // =========================================================================

    /// OR_EqIdM: OR r/m64, imm32 (sign-extended) (memory form)
    /// Bochs: OR_EqIdM
    pub(super) fn or_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = op1_64 | op2_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// OR_EqIdR: OR r64, imm32 (sign-extended) (register form)
    /// Bochs: OR_EqIdR
    pub(super) fn or_eq_id_r(&mut self, instr: &Instruction) {
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = op1_64 | op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// OR_EqGqM: OR r/m64, r64 (memory form)
    /// Bochs: OR_EqGqM — read-modify-write memory, src from register
    pub(super) fn or_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);
        let result = op1_64 | op2_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// OR_GqEqR: OR r64, r64 (register form, load-direction)
    /// Bochs: OR_GqEqR
    pub(super) fn or_gq_eq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1_64 = self.get_gpr64(dst);
        let op2_64 = self.get_gpr64(src);
        let result = op1_64 | op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// OR_GqEqM: OR r64, r/m64 (memory form, load-direction)
    /// Bochs: OR_GqEqM — register destination, memory source
    pub(super) fn or_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_64 = self.read_linear_qword(seg, laddr)?;
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = op1_64 | op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    // =========================================================================
    // AND instructions
    // =========================================================================

    /// AND_EqGqM: AND r/m64, r64 (memory form)
    /// Bochs: AND_EqGqM — read-modify-write memory, src from register
    pub(super) fn and_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let op2_64 = self.get_gpr64(instr.src() as usize);
        let result = op1_64 & op2_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// AND_GqEqR: AND r64, r64 (register form, load-direction)
    /// Bochs: AND_GqEqR
    pub(super) fn and_gq_eq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1_64 = self.get_gpr64(dst);
        let op2_64 = self.get_gpr64(src);
        let result = op1_64 & op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// AND_GqEqM: AND r64, r/m64 (memory form, load-direction)
    /// Bochs: AND_GqEqM — register destination, memory source
    pub(super) fn and_gq_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op2_64 = self.read_linear_qword(seg, laddr)?;
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = op1_64 & op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// AND_EqIdM: AND r/m64, imm32 (sign-extended) (memory form)
    /// Bochs: AND_EqIdM
    pub(super) fn and_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = op1_64 & op2_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// AND_EqIdR: AND r64, imm32 (sign-extended) (register form)
    /// Bochs: AND_EqIdR
    pub(super) fn and_eq_id_r(&mut self, instr: &Instruction) {
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = op1_64 & op2_64;

        self.set_gpr64(dst, result);
        self.set_flags_oszapc_logic_64(result);
    }

    // =========================================================================
    // NOT instructions
    // =========================================================================

    /// NOT_EqM: NOT r/m64 (memory form)
    /// Bochs: NOT_EqM — bitwise complement; does NOT affect flags
    pub(super) fn not_eq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let (op1_64, rmw_laddr) = self.read_rmw_linear_qword(seg, laddr)?;
        let result = !op1_64;

        self.write_rmw_linear_qword(rmw_laddr, result);
        Ok(())
    }

    /// NOT_EqR: NOT r64 (register form)
    /// Bochs: NOT_EqR — bitwise complement; does NOT affect flags
    pub(super) fn not_eq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let result = !op1_64;

        self.set_gpr64(dst, result);
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST_EqGqR: TEST r/m64, r64 (register form)
    /// Bochs: TEST_EqGqR — AND without storing result; sets flags only
    pub(super) fn test_eq_gq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1_64 = self.get_gpr64(dst);
        let op2_64 = self.get_gpr64(src);
        let result = op1_64 & op2_64;

        self.set_flags_oszapc_logic_64(result);
    }

    /// TEST_EqGqM: TEST r/m64, r64 (memory form)
    /// Bochs: TEST_EqGqM — memory source ANDed with register, sets flags only
    pub(super) fn test_eq_gq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1_64 = self.read_linear_qword(seg, laddr)?;
        // In Bochs TEST_EqGqM, i->src() = nnn (register operand).
        // Our decoder ELSE branch: dst=nnn, src1=rm. So use dst() for the register.
        let op2_64 = self.get_gpr64(instr.dst() as usize);
        let result = op1_64 & op2_64;

        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    /// TEST_EqIdR: TEST r/m64, imm32 (sign-extended) (register form)
    /// Bochs: TEST_EqIdR — imm32 sign-extended to 64 bits
    pub(super) fn test_eq_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1_64 = self.get_gpr64(dst);
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1_64 & op2_64;

        self.set_flags_oszapc_logic_64(result);
    }

    /// TEST_EqIdM: TEST r/m64, imm32 (sign-extended) (memory form)
    /// Bochs: TEST_EqIdM — imm32 sign-extended to 64 bits
    pub(super) fn test_eq_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr64(instr);
        let seg = BxSegregs::from(instr.seg());
        let seg_idx = seg as usize;
        let laddr = self.get_laddr64(seg_idx, eaddr);
        let op1_64 = self.read_linear_qword(seg, laddr)?;
        let op2_64 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1_64 & op2_64;

        self.set_flags_oszapc_logic_64(result);
        Ok(())
    }

    // =========================================================================
    // CMP_EqGqR — register-form of CMP r/m64, r64
    // (arith64.rs has CMP_EqGqM, CMP_GqEqR, CMP_GqEqM, CMP_EqIdR, CMP_EqIdM)
    // =========================================================================

    /// CMP_EqGqR: CMP r/m64, r64 (register form, store-direction)
    /// Bochs: CMP_EqGqR — compare reg with reg, sets flags only
    pub(super) fn cmp_eq_gq_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr64(dst);
        let op2 = self.get_gpr64(src);
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, result);
    }

    // =========================================================================
    // CMP RAX, imm32 (sign-extended to 64) — accumulator form
    // =========================================================================

    /// CMP_RAXId: CMP RAX, imm32 (sign-extended to 64)
    /// Bochs: CMP_RAXId — accumulator form for 64-bit CMP with sign-extended imm32
    pub(super) fn cmp_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1.wrapping_sub(op2);
        self.update_flags_sub64(op1, op2, result);
    }

    // =========================================================================
    // Accumulator-immediate forms (RAX, imm32 sign-extended to 64)
    // =========================================================================

    /// OR RAX, imm32 (sign-extended to 64) — Bochs OR_RAXId → OR_EqIdR
    /// Accumulator form: hardcodes RAX as destination (decoder stores rm != 0).
    pub(super) fn or_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1 | op2;
        self.set_gpr64(0, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// AND RAX, imm32 (sign-extended to 64) — Bochs AND_RAXId → AND_EqIdR
    pub(super) fn and_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1 & op2;
        self.set_gpr64(0, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// XOR RAX, imm32 (sign-extended to 64) — Bochs XOR_RAXId → XOR_EqIdR
    pub(super) fn xor_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1 ^ op2;
        self.set_gpr64(0, result);
        self.set_flags_oszapc_logic_64(result);
    }

    /// TEST RAX, imm32 (sign-extended to 64) — Bochs TEST_RAXId → TEST_EqIdR
    /// No writeback — sets flags only.
    pub(super) fn test_rax_id(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr64(0); // RAX
        let op2 = instr.id() as i32 as u64; // sign-extend imm32 to 64 bits
        let result = op1 & op2;
        self.set_flags_oszapc_logic_64(result);
    }

    // =========================================================================
    // Unified dispatch handlers (R vs M)
    // =========================================================================

    /// XOR r/m64, r64 - unified dispatch
    /// Store-direction: decoder swaps [0]=rm=DEST, [1]=nnn=SRC
    pub(super) fn xor_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.operands.dst as usize);
            let op2 = self.get_gpr64(instr.operands.src1 as usize);
            let result = op1 ^ op2;
            self.set_gpr64(instr.operands.dst as usize, result);
            self.set_flags_oszapc_logic_64(result);
            Ok(())
        } else {
            self.xor_eq_gq_m(instr)
        }
    }

    /// XOR r64, r/m64 - unified dispatch
    pub(super) fn xor_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_gq_eq_r(instr);
            Ok(())
        } else {
            self.xor_gq_eq_m(instr)
        }
    }

    /// XOR r/m64, imm32 - unified dispatch
    pub(super) fn xor_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_eq_id_r(instr);
            Ok(())
        } else {
            self.xor_eq_id_m(instr)
        }
    }

    /// OR r/m64, r64 - unified dispatch
    /// Store-direction: decoder swaps [0]=rm=DEST, [1]=nnn=SRC
    pub(super) fn or_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.operands.dst as usize);
            let op2 = self.get_gpr64(instr.operands.src1 as usize);
            let result = op1 | op2;
            self.set_gpr64(instr.operands.dst as usize, result);
            self.set_flags_oszapc_logic_64(result);
            Ok(())
        } else {
            self.or_eq_gq_m(instr)
        }
    }

    /// OR r64, r/m64 - unified dispatch
    pub(super) fn or_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_gq_eq_r(instr);
            Ok(())
        } else {
            self.or_gq_eq_m(instr)
        }
    }

    /// OR r/m64, imm32 - unified dispatch
    pub(super) fn or_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_eq_id_r(instr);
            Ok(())
        } else {
            self.or_eq_id_m(instr)
        }
    }

    /// AND r/m64, r64 - unified dispatch
    /// Store-direction: decoder swaps [0]=rm=DEST, [1]=nnn=SRC
    pub(super) fn and_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let op1 = self.get_gpr64(instr.operands.dst as usize);
            let op2 = self.get_gpr64(instr.operands.src1 as usize);
            let result = op1 & op2;
            self.set_gpr64(instr.operands.dst as usize, result);
            self.set_flags_oszapc_logic_64(result);
            Ok(())
        } else {
            self.and_eq_gq_m(instr)
        }
    }

    /// AND r64, r/m64 - unified dispatch
    pub(super) fn and_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_gq_eq_r(instr);
            Ok(())
        } else {
            self.and_gq_eq_m(instr)
        }
    }

    /// AND r/m64, imm32 - unified dispatch
    pub(super) fn and_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_eq_id_r(instr);
            Ok(())
        } else {
            self.and_eq_id_m(instr)
        }
    }

    /// NOT r/m64 - unified dispatch
    pub(super) fn not_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.not_eq_r(instr);
            Ok(())
        } else {
            self.not_eq_m(instr)
        }
    }

    /// TEST r/m64, r64 - unified dispatch
    pub(super) fn test_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_eq_gq_r(instr);
            Ok(())
        } else {
            self.test_eq_gq_m(instr)
        }
    }

    /// TEST r/m64, imm32 - unified dispatch
    pub(super) fn test_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_eq_id_r(instr);
            Ok(())
        } else {
            self.test_eq_id_m(instr)
        }
    }

    /// CMP r64, r/m64 - unified dispatch (GqEq: register dst, register or memory src)
    /// Calls cmp_gq_eq_r / cmp_gq_eq_m from arith64.rs
    pub(super) fn cmp_gq_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_gq_eq_r(instr);
            Ok(())
        } else {
            self.cmp_gq_eq_m(instr)
        }
    }

    /// CMP r/m64, r64 - unified dispatch (EqGq: memory or register op1, register op2)
    /// Register form (cmp_eq_gq_r) is defined here; memory form (cmp_eq_gq_m) from arith64.rs
    pub(super) fn cmp_eq_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_eq_gq_r(instr);
            Ok(())
        } else {
            self.cmp_eq_gq_m(instr)
        }
    }

    /// CMP r/m64, imm32 - unified dispatch
    /// Calls cmp_eq_id_r / cmp_eq_id_m from arith64.rs
    pub(super) fn cmp_eq_id(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_eq_id_r(instr);
            Ok(())
        } else {
            self.cmp_eq_id_m(instr)
        }
    }

    /// INC r/m64 - unified dispatch
    /// Calls inc_eq_r / inc_eq_m from arith64.rs (they preserve CF correctly)
    pub(super) fn inc_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.inc_eq_r(instr);
            Ok(())
        } else {
            self.inc_eq_m(instr)
        }
    }

    /// DEC r/m64 - unified dispatch
    /// Calls dec_eq_r / dec_eq_m from arith64.rs (they preserve CF correctly)
    pub(super) fn dec_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.dec_eq_r(instr);
            Ok(())
        } else {
            self.dec_eq_m(instr)
        }
    }

    // =========================================================================
    // ZERO_IDIOM — XOR register with itself (64-bit zero idiom)
    // =========================================================================

    /// ZERO_IDIOM_GqR: XOR r64, r64 where src==dst (zero idiom)
    /// Sets register to 0 and sets flags for zero result.
    pub(super) fn zero_idiom_gq_r(&mut self, instr: &Instruction) {
        let dst = instr.operands.dst as usize;
        self.set_gpr64(dst, 0);
        self.set_flags_oszapc_logic_64(0);
    }
}
