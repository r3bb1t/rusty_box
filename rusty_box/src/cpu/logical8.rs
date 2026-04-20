//! 8-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical8.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction, BX_64BIT_REG_RIP},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 8-bit logical operations (AND, OR, XOR, TEST)
    pub fn set_flags_oszapc_logic_8(&mut self, result: u8) {
        // Clear OF, CF (always 0 for logical operations)
        // Set SF, ZF, PF based on result
        // AF is undefined
        let sf = (result & 0x80) != 0;
        let zf = result == 0;
        let pf = result.count_ones().is_multiple_of(2);

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
        // OF=0, CF=0 are already cleared
        self.oszapc.set_oszapc_logic_8(result);
    }

    /// Update flags for 8-bit subtraction (CMP, SUB)
    pub fn set_flags_oszapc_sub_8(&mut self, op1: u8, op2: u8, result: u8) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x80) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x80) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = result.count_ones().is_multiple_of(2);

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
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

    /// Read 8-bit register with extend8bitL support (matches BX_READ_8BIT_REGx)
    ///
    /// Bochs macro: ext ? gen_reg[index].rl : (index<4 ? gen_reg[index].rl : gen_reg[index-4].rh)
    /// When REX present (extend8bit_l != 0): indices 4-7 = SPL/BPL/SIL/DIL (low byte of RSP-RDI)
    /// Without REX: indices 4-7 = AH/CH/DH/BH (high byte of RAX-RBX)
    pub fn read_8bit_regx(&self, reg_idx: usize, extend8bit_l: u8) -> u8 {
        if extend8bit_l != 0 || (reg_idx & 4) == 0 {
            // REX present OR index 0-3: low byte of gen_reg[index]
            self.gen_reg[reg_idx].rl()
        } else {
            // No REX, index 4-7: high byte of gen_reg[index-4] (AH/CH/DH/BH)
            let reg16_idx = reg_idx & 0x3;
            (self.get_gpr16(reg16_idx) >> 8) as u8
        }
    }

    /// Write 8-bit register with extend8bitL support (matches BX_WRITE_8BIT_REGx)
    pub fn write_8bit_regx(&mut self, reg_idx: usize, extend8bit_l: u8, val: u8) {
        if extend8bit_l != 0 || (reg_idx & 4) == 0 {
            // REX present OR index 0-3: low byte of gen_reg[index]
            self.gen_reg[reg_idx].set_rl(val);
        } else {
            // No REX, index 4-7: high byte of gen_reg[index-4] (AH/CH/DH/BH)
            let reg16_idx = reg_idx & 0x3;
            let current = self.get_gpr16(reg16_idx);
            let new_val = (current & 0x00FF) | ((val as u16) << 8);
            self.set_gpr16(reg16_idx, new_val);
        }
    }

    /// Mode-dispatching address resolver (matches BX_CPU_RESOLVE_ADDR).
    /// Returns 64-bit effective address in long mode, zero-extended 32-bit otherwise.
    #[inline]
    pub fn resolve_addr(&self, instr: &Instruction) -> u64 {
        // Bochs BX_CPU_RESOLVE_ADDR: (i)->as64L() ? BxResolve64 : BxResolve32
        // Must use per-instruction address-size attribute, NOT CPU mode.
        // In 64-bit mode with 67h prefix, as64_l()==0 → use 32-bit resolution.
        if instr.as64_l() != 0 {
            self.resolve_addr64(instr)
        } else {
            u64::from(self.resolve_addr32(instr))
        }
    }

    /// Resolve effective address (matches BX_CPU_RESOLVE_ADDR)
    #[inline]
    pub fn resolve_addr32(&self, instr: &Instruction) -> u32 {
        let base_reg = instr.sib_base() as usize;
        let mut eaddr = if base_reg < 16 {
            self.get_gpr32(base_reg)
        } else if base_reg == BX_64BIT_REG_RIP {
            // RIP-relative addressing (64-bit mode only, mod=0 rm=5).
            // gen_reg[RIP] already advanced by ilen before execution.
            // Truncate to u32 — works for addresses below 4GB.
            self.gen_reg[BX_64BIT_REG_RIP].rrx() as u32
        } else {
            0
        };

        eaddr = eaddr.wrapping_add(instr.displ32s() as u32);

        let index_reg = instr.sib_index();
        if index_reg != 4 {
            let index_val = if index_reg < 16 {
                self.get_gpr32(index_reg as usize)
            } else {
                0
            };
            let scale = instr.sib_scale();
            eaddr = eaddr.wrapping_add(index_val << scale);
        }

        if instr.as32_l() == 0 {
            eaddr & 0xFFFF
        } else {
            eaddr
        }
    }

    /// Write-back phase of a read-modify-write byte access.
    /// Uses address_xlation populated by read_rmw_virtual_byte.
    /// Bochs: write_RMW_linear_byte (access2.cc)
    #[inline]
    pub fn write_rmw_linear_byte(&mut self, val: u8) {
        if self.address_xlation.pages > 2 {
            // Host pointer cached from TLB hit — direct write (fastest path)
            self.address_xlation.write_pages_u8(val);
        } else {
            // pages == 1: single-page physical write
            self.mem_write_byte(self.address_xlation.paddress1, val);
        }
    }

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
