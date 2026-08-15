//! 8-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical8.cc

use super::{
    cpu::BxCpuC,
    decoder::{BxSegregs, Instruction},
};

impl<T: crate::cpu::instrumentation::Instrumentation> BxCpuC<T> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 8-bit logical operations (AND, OR, XOR, TEST).
    /// Bochs SET_FLAGS_OSZAPC_LOGIC_8: clears OF/CF/AF, sets SF/ZF/PF from result.
    pub fn set_flags_oszapc_logic_8(&mut self, result: u8) {
        self.oszapc.set_oszapc_logic_8(result);
    }

    /// Update flags for 8-bit subtraction (CMP, SUB).
    /// Bochs SET_FLAGS_OSZAPC_SUB_8.
    pub fn set_flags_oszapc_sub_8(&mut self, op1: u8, op2: u8, result: u8) {
        self.oszapc.set_oszapc_sub_8(op1, op2, result);
    }

    // =========================================================================
    // XOR instructions
    // =========================================================================

    /// XOR_EbIbR: XOR r/m8, imm8 (register form)
    /// Opcode: 0x80/6 (8-bit)
    /// Matches BX_CPU_C::XOR_EbIbR
    pub fn xor_eb_ib_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = instr.ib();
        let result = op1 ^ op2;

        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// XOR_ALIb: XOR AL, imm8
    /// Dedicated handler for opcode 0x34 - accumulator-immediate form
    /// Must hardcode AL (register 0) because the decoder sets dst from opcode
    /// low bits (b1 & 7 = 4 for opcode 0x34), which would be AH, not AL.
    pub fn xor_al_ib(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1 ^ op2;

        self.set_gpr8(0, result); // AL
        self.set_flags_oszapc_logic_8(result);
    }

    /// XOR_GbEbR: XOR r8, r/m8 (register form)
    /// Matches BX_CPU_C::XOR_GbEbR
    pub fn xor_gb_eb_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = self.read_8bit_regx(src, extend8bit_l);
        let result = op1 ^ op2;

        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// XOR_EbGbR: XOR r/m8, r8 (register form, store-direction)
    /// Opcode 0x30: reg (operands.dst) = SOURCE, rm (operands.src1) = DESTINATION
    pub fn xor_eb_gb_r(&mut self, instr: &Instruction) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.src1() as usize, extend8bit_l); // rm = destination
        let op2 = self.read_8bit_regx(instr.dst() as usize, extend8bit_l); // reg = source
        let result = op1 ^ op2;
        self.write_8bit_regx(instr.src1() as usize, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    // =========================================================================
    // Helper functions
    // =========================================================================











    // =========================================================================
    // CMP instructions
    // =========================================================================

    /// CMP r8, r8
    pub fn cmp_gb_eb_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = self.read_8bit_regx(src, extend8bit_l);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    /// CMP AL, imm8
    pub fn cmp_al_ib(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    /// CMP_GbEb_M: CMP r8, r/m8 (memory form)
    pub fn cmp_gb_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let op2 = self.v_read_byte(seg, eaddr)?;
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
        Ok(())
    }

    /// CMP_EbGb_M: CMP r/m8, r8 (memory form)
    pub fn cmp_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_byte(seg, eaddr)?;
        let op2 = self.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg field = source
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
        Ok(())
    }

    /// CMP_EbIb_M: CMP r/m8, imm8 (memory form)
    pub fn cmp_eb_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_byte(seg, eaddr)?;
        let op2 = instr.ib();
        // diagnostics disabled for performance
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
        Ok(())
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST r8, r8
    pub fn test_eb_gb_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = self.read_8bit_regx(src, extend8bit_l);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_8(result);
    }

    /// TEST AL, imm8
    pub fn test_al_ib(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_8(result);
    }

    /// TEST_EbIbR: TEST r8, imm8 (register form)
    /// Matches BX_CPU_C::TEST_EbIbR
    pub fn test_eb_ib_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = instr.ib();
        let result = op1 & op2;

        self.set_flags_oszapc_logic_8(result);
    }

    // =========================================================================
    // AND instructions
    // =========================================================================

    /// AND r8, r8
    pub fn and_gb_eb_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = self.read_8bit_regx(src, extend8bit_l);
        let result = op1 & op2;
        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// AND_EbGbR: AND r/m8, r8 (register form, store-direction)
    /// Opcode 0x20: reg (operands.dst) = SOURCE, rm (operands.src1) = DESTINATION
    pub fn and_eb_gb_r(&mut self, instr: &Instruction) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.src1() as usize, extend8bit_l); // rm = destination
        let op2 = self.read_8bit_regx(instr.dst() as usize, extend8bit_l); // reg = source
        let result = op1 & op2;
        self.write_8bit_regx(instr.src1() as usize, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// AND AL, imm8
    pub fn and_al_ib(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr8(0);
        let op2 = instr.ib();
        let result = op1 & op2;
        self.set_gpr8(0, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// AND_EbIbR: AND r8, imm8 (register form)
    /// Matches BX_CPU_C::AND_EbIbR
    pub fn and_eb_ib_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = instr.ib();
        let result = op1 & op2;

        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    // =========================================================================
    // OR instructions
    // =========================================================================

    /// OR_EbGbR: OR r/m8, r8 (register form, store-direction)
    /// Opcode 0x08: reg (operands.dst) = SOURCE, rm (operands.src1) = DESTINATION
    pub fn or_eb_gb_r(&mut self, instr: &Instruction) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.src1() as usize, extend8bit_l); // rm = destination
        let op2 = self.read_8bit_regx(instr.dst() as usize, extend8bit_l); // reg = source
        let result = op1 | op2;
        self.write_8bit_regx(instr.src1() as usize, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR r8, r8 (load-direction, opcode 0x0A)
    pub fn or_gb_eb_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = self.read_8bit_regx(src, extend8bit_l);
        let result = op1 | op2;
        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR AL, imm8
    pub fn or_al_ib(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr8(0);
        let op2 = instr.ib();
        let result = op1 | op2;
        self.set_gpr8(0, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR_EbIbR: OR r8, imm8 (register form)
    /// Matches BX_CPU_C::OR_EbIbR
    pub fn or_eb_ib_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = instr.ib();
        let result = op1 | op2;

        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    // =========================================================================
    // NOT instructions
    // =========================================================================

    /// NOT r8
    pub fn not_eb_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        self.write_8bit_regx(dst, extend8bit_l, !op1);
        // NOT does not affect flags
    }

    /// NOT r/m8 (memory form)
    /// Matches BX_CPU_C::NOT_EbM
    pub fn not_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_8 = self.v_read_rmw_byte(seg, eaddr)?;
        let result = !op1_8;

        self.write_rmw_linear_byte(result);
        Ok(())
    }

    // =========================================================================
    // Memory-form instructions
    // =========================================================================

    /// XOR_EbGbM: XOR r/m8, r8 (memory form)
    /// Matches BX_CPU_C::XOR_EbGbM
    pub fn xor_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_byte(seg, eaddr)?;
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 ^ op2;

        self.write_rmw_linear_byte(result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// XOR_GbEbM: XOR r8, r/m8 (memory form)
    /// Matches BX_CPU_C::XOR_GbEbM
    pub fn xor_gb_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2 = self.v_read_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst_reg, extend8bit_l);
        let result = op1 ^ op2;

        self.write_8bit_regx(dst_reg, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// XOR_EbIbM: XOR r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::XOR_EbIbM
    pub fn xor_eb_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_byte(seg, eaddr)?;
        let op2 = instr.ib();
        let result = op1 ^ op2;

        self.write_rmw_linear_byte(result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// OR_EbGbM: OR r/m8, r8 (memory form)
    /// Matches BX_CPU_C::OR_EbGbM
    pub fn or_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_byte(seg, eaddr)?;
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 | op2;

        self.write_rmw_linear_byte(result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// OR_GbEbM: OR r8, r/m8 (memory form)
    /// Matches BX_CPU_C::OR_GbEbM
    pub fn or_gb_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2 = self.v_read_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst_reg, extend8bit_l);
        let result = op1 | op2;

        self.write_8bit_regx(dst_reg, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// OR_EbIbM: OR r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::OR_EbIbM
    pub fn or_eb_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_byte(seg, eaddr)?;
        let op2 = instr.ib();
        let result = op1 | op2;

        self.write_rmw_linear_byte(result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// AND_EbGbM: AND r/m8, r8 (memory form)
    /// Matches BX_CPU_C::AND_EbGbM
    pub fn and_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_byte(seg, eaddr)?;
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 & op2;

        self.write_rmw_linear_byte(result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// AND_GbEbM: AND r8, r/m8 (memory form)
    /// Matches BX_CPU_C::AND_GbEbM
    pub fn and_gb_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2 = self.v_read_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst_reg, extend8bit_l);
        let result = op1 & op2;

        self.write_8bit_regx(dst_reg, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// AND_EbIbM: AND r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::AND_EbIbM
    pub fn and_eb_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_rmw_byte(seg, eaddr)?;
        let op2 = instr.ib();
        let result = op1 & op2;

        self.write_rmw_linear_byte(result);
        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// TEST_EbGbM: TEST r/m8, r8 (memory form)
    /// Matches BX_CPU_C::TEST_EbGbM
    pub fn test_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_byte(seg, eaddr)?;
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 & op2;

        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    /// TEST_EbIbM: TEST r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::TEST_EbIbM
    pub fn test_eb_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_byte(seg, eaddr)?;
        let op2 = instr.ib();
        let result = op1 & op2;

        self.set_flags_oszapc_logic_8(result);
        Ok(())
    }

    // =========================================================================
    // CMP register-form instructions (needed for unified dispatchers)
    // =========================================================================

    /// CMP_EbGbR: CMP r/m8, r8 (register form)
    /// Opcode 0x38: reg (dst()) = second operand, rm (src()) = first operand
    pub fn cmp_eb_gb_r(&mut self, instr: &Instruction) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.src() as usize, extend8bit_l); // rm = first operand
        let op2 = self.read_8bit_regx(instr.dst() as usize, extend8bit_l); // reg = second operand
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    /// CMP_EbIbR: CMP r/m8, imm8 (register form)
    /// Matches BX_CPU_C::CMP_EbIbR
    pub fn cmp_eb_ib_r(&mut self, instr: &Instruction) {
        let op1 = self.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let op2 = instr.ib();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    // =========================================================================
    // Unified handlers: dispatch R/M based on instr.mod_c0()
    // =========================================================================

    pub fn and_eb_gb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_eb_gb_r(instr);
            Ok(())
        } else {
            self.and_eb_gb_m(instr)
        }
    }
    pub fn and_gb_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_gb_eb_r(instr);
            Ok(())
        } else {
            self.and_gb_eb_m(instr)
        }
    }
    pub fn and_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_eb_ib_r(instr);
            Ok(())
        } else {
            self.and_eb_ib_m(instr)
        }
    }
    pub fn or_eb_gb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_eb_gb_r(instr);
            Ok(())
        } else {
            self.or_eb_gb_m(instr)
        }
    }
    pub fn or_gb_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_gb_eb_r(instr);
            Ok(())
        } else {
            self.or_gb_eb_m(instr)
        }
    }
    pub fn or_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_eb_ib_r(instr);
            Ok(())
        } else {
            self.or_eb_ib_m(instr)
        }
    }
    pub fn xor_eb_gb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_eb_gb_r(instr);
            Ok(())
        } else {
            self.xor_eb_gb_m(instr)
        }
    }
    pub fn xor_gb_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_gb_eb_r(instr);
            Ok(())
        } else {
            self.xor_gb_eb_m(instr)
        }
    }
    pub fn xor_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_eb_ib_r(instr);
            Ok(())
        } else {
            self.xor_eb_ib_m(instr)
        }
    }
    pub fn not_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.not_eb_r(instr);
            Ok(())
        } else {
            self.not_eb_m(instr)
        }
    }
    pub fn test_eb_gb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_eb_gb_r(instr);
            Ok(())
        } else {
            self.test_eb_gb_m(instr)
        }
    }
    pub fn test_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_eb_ib_r(instr);
            Ok(())
        } else {
            self.test_eb_ib_m(instr)
        }
    }
    pub fn cmp_gb_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_gb_eb_r(instr);
            Ok(())
        } else {
            self.cmp_gb_eb_m(instr)
        }
    }
    pub fn cmp_eb_gb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_eb_gb_r(instr);
            Ok(())
        } else {
            self.cmp_eb_gb_m(instr)
        }
    }
    pub fn cmp_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_eb_ib_r(instr);
            Ok(())
        } else {
            self.cmp_eb_ib_m(instr)
        }
    }
}
