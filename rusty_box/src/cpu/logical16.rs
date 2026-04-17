//! 16-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical16.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 16-bit logical operations
    pub fn set_flags_oszapc_logic_16(&mut self, result: u16) {
        let sf = (result & 0x8000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones().is_multiple_of(2);

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

    /// Update flags for 16-bit subtraction
    pub fn set_flags_oszapc_sub_16(&mut self, op1: u16, op2: u16, result: u16) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x8000) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones().is_multiple_of(2);

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
    }

    /// Update flags for INC (preserves CF)
    pub fn set_flags_oszap_inc_16(&mut self, result: u16, op1: u16) {
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = result == 0x8000; // Only overflow when 0x7FFF -> 0x8000
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones().is_multiple_of(2);

        // CF is not affected by INC
        const OSZAP: EFlags = EFlags::PF
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        self.eflags.remove(OSZAP);

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
    }

    /// Update flags for DEC (preserves CF)
    pub fn set_flags_oszap_dec_16(&mut self, result: u16, op1: u16) {
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = result == 0x7FFF && op1 == 0x8000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones().is_multiple_of(2);

        const OSZAP: EFlags = EFlags::PF
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::OF);
        self.eflags.remove(OSZAP);

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
    }

    // =========================================================================
    // ZERO_IDIOM - XOR register with itself (optimization: set to 0)
    // =========================================================================

    /// ZERO_IDIOM_GwR: XOR r16, r16 (zero idiom - set register to 0)
    /// Opcode: XOR_EwGw_ZERO_IDIOM or XOR_GwEw_ZERO_IDIOM
    /// Matches BX_CPU_C::ZERO_IDIOM_GwR
    pub fn zero_idiom_gw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        self.set_gpr16(dst, 0);
        self.set_flags_oszapc_logic_16(0);
    }

    // =========================================================================
    // CMP instructions
    // =========================================================================

    /// CMP r16, r16
    pub fn cmp_gw_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
    }

    /// CMP AX, imm16
    pub fn cmp_ax_iw(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
    }

    /// CMP r/m16, imm16
    pub fn cmp_ew_iw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
    }

    /// CMP_GwEw_M: CMP r16, r/m16 (memory form)
    pub fn cmp_gw_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.get_gpr16(instr.dst() as usize);
        let op2 = self.v_read_word(seg, eaddr)?;
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        Ok(())
    }

    /// CMP_EwIw_M: CMP r/m16, imm16 (memory form)
    pub fn cmp_ew_iw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.v_read_word(seg, eaddr)?;
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        Ok(())
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST r16, r16
    pub fn test_ew_gw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
    }

    /// TEST AX, imm16
    pub fn test_ax_iw(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
    }

    /// TEST r/m16, imm16
    pub fn test_ew_iw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
    }

    // =========================================================================
    // AND instructions
    // =========================================================================

    /// AND r16, r16
    pub fn and_gw_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1 & op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// AND_EwGwR: AND r/m16, r16 (register form, store-direction)
    /// Opcode 0x21: decoder swaps: [0]=rm=DEST, [1]=nnn=SOURCE
    pub fn and_ew_gw_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(instr.dst() as usize); // rm = destination
        let op2 = self.get_gpr16(instr.src1() as usize); // nnn = source
        let result = op1 & op2;
        self.set_gpr16(instr.dst() as usize, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// AND AX, imm16
    pub fn and_ax_iw(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(0);
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// AND r/m16, imm16
    pub fn and_ew_iw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    // =========================================================================
    // XOR instructions
    // =========================================================================

    /// XOR_GwEwR: XOR r16, r16 (register form)
    /// Matches BX_CPU_C::XOR_GwEwR
    pub fn xor_gw_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1 ^ op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// XOR_EwGwR: XOR r/m16, r16 (register form, store-direction)
    /// Opcode 0x31: decoder swaps: [0]=rm=DEST, [1]=nnn=SOURCE
    pub fn xor_ew_gw_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(instr.dst() as usize); // rm = destination
        let op2 = self.get_gpr16(instr.src1() as usize); // nnn = source
        let result = op1 ^ op2;
        self.set_gpr16(instr.dst() as usize, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// XOR_EwIwR: XOR r/m16, imm16 (register form)
    /// Matches BX_CPU_C::XOR_EwIwR
    pub fn xor_ew_iw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1 ^ op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// XOR AX, imm16 (opcode 0x35 with 66 prefix) — accumulator-immediate form.
    /// Must hardcode register 0 (AX) because decoder sets rm = opcode & 7 = 5 (BP).
    pub fn xor_ax_iw(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1 ^ op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    // =========================================================================
    // OR instructions
    // =========================================================================

    /// OR r16, r16
    pub fn or_gw_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1 | op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// OR_EwGwR: OR r/m16, r16 (register form, store-direction)
    /// Opcode 0x09: decoder swaps: [0]=rm=DEST, [1]=nnn=SOURCE
    pub fn or_ew_gw_r(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(instr.dst() as usize); // rm = destination
        let op2 = self.get_gpr16(instr.src1() as usize); // nnn = source
        let result = op1 | op2;
        self.set_gpr16(instr.dst() as usize, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// OR AX, imm16
    pub fn or_ax_iw(&mut self, instr: &Instruction) {
        let op1 = self.get_gpr16(0);
        let op2 = instr.iw();
        let result = op1 | op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// OR_EwIwR: OR r/m16, imm16 (register form)
    /// Matches BX_CPU_C::OR_EwIwR
    pub fn or_ew_iw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1 | op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    // =========================================================================
    // NOT instructions
    // =========================================================================

    /// NOT r16
    pub fn not_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        self.set_gpr16(dst, !op1);
    }

    // =========================================================================
    // Helper functions for memory operations
    // =========================================================================

    // resolve_addr32 is defined in logical8.rs to avoid duplicate definitions

    // get_laddr32_seg is defined in logical8.rs to avoid duplicate definitions

    /// Write-back phase of a read-modify-write word access.
    /// Uses address_xlation populated by read_rmw_virtual_word.
    /// Bochs: write_RMW_linear_word (access2.cc)
    #[inline]
    pub fn write_rmw_linear_word(&mut self, val: u16) {
        if self.address_xlation.pages > 2 {
            // Host pointer cached from TLB hit — direct write
            self.address_xlation.write_pages_u16(val);
        } else if self.address_xlation.pages == 1 {
            // Single-page physical write
            self.mem_write_word(self.address_xlation.paddress1, val);
        } else {
            // Cross-page (pages == 2): split write (little-endian)
            let bytes = val.to_le_bytes();
            self.mem_write_byte(self.address_xlation.paddress1, bytes[0]);
            self.mem_write_byte(self.address_xlation.paddress2, bytes[1]);
        }
    }

    // =========================================================================
    // Memory-form instructions
    // =========================================================================

    /// XOR_EwGwM: XOR r/m16, r16 (memory form)
    /// Matches BX_CPU_C::XOR_EwGwM
    pub fn xor_ew_gw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 ^ op2_16;

        self.write_rmw_linear_word(result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// XOR_GwEwM: XOR r16, r/m16 (memory form)
    /// Matches BX_CPU_C::XOR_GwEwM
    pub fn xor_gw_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.v_read_word(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_16 = self.get_gpr16(dst_reg);
        let result = op1_16 ^ op2_16;

        self.set_gpr16(dst_reg, result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// XOR_EwIwM: XOR r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::XOR_EwIwM
    pub fn xor_ew_iw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let op2_16 = instr.iw();
        let result = op1_16 ^ op2_16;

        self.write_rmw_linear_word(result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// OR_EwGwM: OR r/m16, r16 (memory form)
    /// Matches BX_CPU_C::OR_EwGwM
    pub fn or_ew_gw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 | op2_16;

        self.write_rmw_linear_word(result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// OR_GwEwM: OR r16, r/m16 (memory form)
    /// Matches BX_CPU_C::OR_GwEwM
    pub fn or_gw_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.v_read_word(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_16 = self.get_gpr16(dst_reg);
        let result = op1_16 | op2_16;

        self.set_gpr16(dst_reg, result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// OR_EwIwM: OR r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::OR_EwIwM
    pub fn or_ew_iw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let op2_16 = instr.iw();
        let result = op1_16 | op2_16;

        self.write_rmw_linear_word(result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// AND_EwGwM: AND r/m16, r16 (memory form)
    /// Matches BX_CPU_C::AND_EwGwM
    pub fn and_ew_gw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 & op2_16;

        self.write_rmw_linear_word(result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// AND_GwEwM: AND r16, r/m16 (memory form)
    /// Matches BX_CPU_C::AND_GwEwM
    pub fn and_gw_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.v_read_word(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let op1_16 = self.get_gpr16(dst_reg);
        let result = op1_16 & op2_16;

        self.set_gpr16(dst_reg, result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// AND_EwIwM: AND r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::AND_EwIwM
    pub fn and_ew_iw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let op2_16 = instr.iw();
        let result = op1_16 & op2_16;

        self.write_rmw_linear_word(result);
        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// NOT_EwM: NOT r/m16 (memory form)
    /// Matches BX_CPU_C::NOT_EwM
    pub fn not_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_rmw_word(seg, eaddr)?;
        let result = !op1_16;

        self.write_rmw_linear_word(result);
        Ok(())
    }

    /// TEST_EwGwM: TEST r/m16, r16 (memory form)
    /// Opcode 0x85 is NOT store-direction, so dst() = nnn = register operand
    pub fn test_ew_gw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_word(seg, eaddr)?;
        let src_reg = instr.dst() as usize; // Opcode 0x85 is NOT store-direction: dst()=nnn=register
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 & op2_16;

        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    /// TEST_EwIwM: TEST r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::TEST_EwIwM
    pub fn test_ew_iw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.v_read_word(seg, eaddr)?;
        let op2_16 = instr.iw();
        let result = op1_16 & op2_16;

        self.set_flags_oszapc_logic_16(result);
        Ok(())
    }

    // =========================================================================
    // Unified handlers: dispatch R/M based on instr.mod_c0()
    // =========================================================================

    pub fn xor_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_ew_gw_r(instr);
            Ok(())
        } else {
            self.xor_ew_gw_m(instr)
        }
    }
    pub fn xor_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_gw_ew_r(instr);
            Ok(())
        } else {
            self.xor_gw_ew_m(instr)
        }
    }
    pub fn xor_ew_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xor_ew_iw_r(instr);
            Ok(())
        } else {
            self.xor_ew_iw_m(instr)
        }
    }
    pub fn and_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_ew_gw_r(instr);
            Ok(())
        } else {
            self.and_ew_gw_m(instr)
        }
    }
    pub fn and_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_gw_ew_r(instr);
            Ok(())
        } else {
            self.and_gw_ew_m(instr)
        }
    }
    pub fn and_ew_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.and_ew_iw_r(instr);
            Ok(())
        } else {
            self.and_ew_iw_m(instr)
        }
    }
    pub fn or_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_ew_gw_r(instr);
            Ok(())
        } else {
            self.or_ew_gw_m(instr)
        }
    }
    pub fn or_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_gw_ew_r(instr);
            Ok(())
        } else {
            self.or_gw_ew_m(instr)
        }
    }
    pub fn or_ew_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.or_ew_iw_r(instr);
            Ok(())
        } else {
            self.or_ew_iw_m(instr)
        }
    }
    pub fn not_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.not_ew_r(instr);
            Ok(())
        } else {
            self.not_ew_m(instr)
        }
    }
    pub fn test_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_ew_gw_r(instr);
            Ok(())
        } else {
            self.test_ew_gw_m(instr)
        }
    }
    pub fn test_ew_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.test_ew_iw_r(instr);
            Ok(())
        } else {
            self.test_ew_iw_m(instr)
        }
    }
    pub fn cmp_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_gw_ew_r(instr);
            Ok(())
        } else {
            self.cmp_gw_ew_m(instr)
        }
    }
    pub fn cmp_ew_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.cmp_ew_iw_r(instr);
            Ok(())
        } else {
            self.cmp_ew_iw_m(instr)
        }
    }

    // =========================================================================
    // Bit Test instructions (BT, BTS, BTR, BTC) — 16-bit
    // Based on Bochs cpu/bit16.cc
    // =========================================================================

    /// BT r/m16, imm8 — Bit Test (0F BA /4 ib, 66h prefix)
    pub fn bt_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = if instr.mod_c0() {
            self.get_gpr16(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, eaddr)?
        };
        let bit = (instr.ib() & 0x0F) as u16;
        let cf = (op1 >> bit) & 1;
        self.eflags.set(EFlags::CF, cf != 0);
        Ok(())
    }

    /// BTS r/m16, imm8 — Bit Test and Set (0F BA /5 ib, 66h prefix)
    pub fn bts_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit = (instr.ib() & 0x0F) as u16;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr16(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr16(dst, op1 | (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_word(seg, eaddr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_word(op1 | (1 << bit));
        }
        Ok(())
    }

    /// BTR r/m16, imm8 — Bit Test and Reset (0F BA /6 ib, 66h prefix)
    pub fn btr_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit = (instr.ib() & 0x0F) as u16;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr16(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr16(dst, op1 & !(1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_word(seg, eaddr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_word(op1 & !(1 << bit));
        }
        Ok(())
    }

    /// BTC r/m16, imm8 — Bit Test and Complement (0F BA /7 ib, 66h prefix)
    pub fn btc_ew_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let bit = (instr.ib() & 0x0F) as u16;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr16(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr16(dst, op1 ^ (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_word(seg, eaddr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_word(op1 ^ (1 << bit));
        }
        Ok(())
    }

    /// BT r/m16, r16 — Bit Test (0F A3, 66h prefix)
    pub fn bt_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr16(instr.src() as usize);
        let op1 = if instr.mod_c0() {
            self.get_gpr16(instr.dst() as usize)
        } else {
            let eaddr = self.resolve_addr(instr);
            // For memory form, bit offset is full op2 (not masked to 15).
            // Bochs bit16.cc: displacement = ((Bit16s)(op2 & 0xfff0)) / 16 * 2
            let displacement = ((op2 as i16) >> 4) << 1;
            let addr = (eaddr as i32).wrapping_add(displacement as i32) as u32;
            let seg = BxSegregs::from(instr.seg());
            self.v_read_word(seg, addr)?
        };
        let bit = op2 & 0x0F;
        let cf = (op1 >> bit) & 1;
        self.eflags.set(EFlags::CF, cf != 0);
        Ok(())
    }

    /// BTS r/m16, r16 — Bit Test and Set (0F AB, 66h prefix)
    pub fn bts_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr16(instr.src() as usize);
        let bit = op2 & 0x0F;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr16(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr16(dst, op1 | (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let displacement = ((op2 as i16) >> 4) << 1;
            let addr = (eaddr as i32).wrapping_add(displacement as i32) as u32;
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_word(seg, addr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_word(op1 | (1 << bit));
        }
        Ok(())
    }

    /// BTR r/m16, r16 — Bit Test and Reset (0F B3, 66h prefix)
    pub fn btr_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr16(instr.src() as usize);
        let bit = op2 & 0x0F;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr16(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr16(dst, op1 & !(1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let displacement = ((op2 as i16) >> 4) << 1;
            let addr = (eaddr as i32).wrapping_add(displacement as i32) as u32;
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_word(seg, addr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_word(op1 & !(1 << bit));
        }
        Ok(())
    }

    /// BTC r/m16, r16 — Bit Test and Complement (0F BB, 66h prefix)
    pub fn btc_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.get_gpr16(instr.src() as usize);
        let bit = op2 & 0x0F;
        if instr.mod_c0() {
            let dst = instr.dst() as usize;
            let op1 = self.get_gpr16(dst);
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.set_gpr16(dst, op1 ^ (1 << bit));
        } else {
            let eaddr = self.resolve_addr(instr);
            let displacement = ((op2 as i16) >> 4) << 1;
            let addr = (eaddr as i32).wrapping_add(displacement as i32) as u32;
            let seg = BxSegregs::from(instr.seg());
            let op1 = self.v_read_rmw_word(seg, addr)?;
            let cf = (op1 >> bit) & 1;
            self.eflags.set(EFlags::CF, cf != 0);
            self.write_rmw_linear_word(op1 ^ (1 << bit));
        }
        Ok(())
    }
}
