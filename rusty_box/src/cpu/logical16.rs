//! 16-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical16.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 16-bit logical operations
    pub fn set_flags_oszapc_logic_16(&mut self, result: u16) {
        let sf = (result & 0x8000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if pf {
            self.eflags |= 1 << 2;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
    }

    /// Update flags for 16-bit subtraction
    pub fn set_flags_oszapc_sub_16(&mut self, op1: u16, op2: u16, result: u16) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x8000) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf {
            self.eflags |= 1 << 0;
        }
        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
    }

    /// Update flags for INC (preserves CF)
    pub fn set_flags_oszap_inc_16(&mut self, result: u16, op1: u16) {
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = result == 0x8000; // Only overflow when 0x7FFF -> 0x8000
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        // CF is not affected by INC
        const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
    }

    /// Update flags for DEC (preserves CF)
    pub fn set_flags_oszap_dec_16(&mut self, result: u16, op1: u16) {
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = result == 0x7FFF && op1 == 0x8000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
    }

    // =========================================================================
    // ZERO_IDIOM - XOR register with itself (optimization: set to 0)
    // =========================================================================

    /// ZERO_IDIOM_GwR: XOR r16, r16 (zero idiom - set register to 0)
    /// Opcode: XOR_EwGw_ZERO_IDIOM or XOR_GwEw_ZERO_IDIOM
    /// Matches BX_CPU_C::ZERO_IDIOM_GwR
    pub fn zero_idiom_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        self.set_gpr16(dst, 0);
        self.set_flags_oszapc_logic_16(0);
    }

    // =========================================================================
    // CMP instructions
    // =========================================================================

    /// CMP r16, r16
    pub fn cmp_gw_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        tracing::trace!(
            "CMP r16, r16: {:#06x} - {:#06x} = {:#06x}",
            op1,
            op2,
            result
        );
    }

    /// CMP AX, imm16
    pub fn cmp_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        tracing::trace!("CMP AX, imm16: {:#06x} - {:#06x}", op1, op2);
    }

    /// CMP r/m16, imm16
    pub fn cmp_ew_iw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        tracing::trace!("CMP r16, imm16: {:#06x} - {:#06x}", op1, op2);
    }

    /// CMP_GwEw_M: CMP r16, r/m16 (memory form)
    pub fn cmp_gw_ew_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.get_gpr16(instr.dst() as usize);
        let op2 = self.read_virtual_word(seg, eaddr);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
    }

    /// CMP_EwIw_M: CMP r/m16, imm16 (memory form)
    pub fn cmp_ew_iw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_virtual_word(seg, eaddr);
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST r16, r16
    pub fn test_ew_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "TEST r16, r16: {:#06x} & {:#06x} = {:#06x}",
            op1,
            op2,
            result
        );
    }

    /// TEST AX, imm16
    pub fn test_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!("TEST AX, imm16: {:#06x} & {:#06x}", op1, op2);
    }

    /// TEST r/m16, imm16
    pub fn test_ew_iw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!("TEST r16, imm16: {:#06x} & {:#06x}", op1, op2);
    }

    // =========================================================================
    // AND instructions
    // =========================================================================

    /// AND r16, r16
    pub fn and_gw_ew_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn and_ew_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(instr.meta_data[0] as usize);  // rm = destination
        let op2 = self.get_gpr16(instr.meta_data[1] as usize);  // nnn = source
        let result = op1 & op2;
        self.set_gpr16(instr.meta_data[0] as usize, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// AND AX, imm16
    pub fn and_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0);
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// AND r/m16, imm16
    pub fn and_ew_iw_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn xor_gw_ew_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn xor_ew_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(instr.meta_data[0] as usize);  // rm = destination
        let op2 = self.get_gpr16(instr.meta_data[1] as usize);  // nnn = source
        let result = op1 ^ op2;
        self.set_gpr16(instr.meta_data[0] as usize, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// XOR_EwIwR: XOR r/m16, imm16 (register form)
    /// Matches BX_CPU_C::XOR_EwIwR
    pub fn xor_ew_iw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = instr.iw();
        let result = op1 ^ op2;
        self.set_gpr16(dst, result);
        self.set_flags_oszapc_logic_16(result);
    }

    // =========================================================================
    // OR instructions
    // =========================================================================

    /// OR r16, r16
    pub fn or_gw_ew_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn or_ew_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(instr.meta_data[0] as usize);  // rm = destination
        let op2 = self.get_gpr16(instr.meta_data[1] as usize);  // nnn = source
        let result = op1 | op2;
        self.set_gpr16(instr.meta_data[0] as usize, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// OR AX, imm16
    pub fn or_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0);
        let op2 = instr.iw();
        let result = op1 | op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// OR_EwIwR: OR r/m16, imm16 (register form)
    /// Matches BX_CPU_C::OR_EwIwR
    pub fn or_ew_iw_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn not_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        self.set_gpr16(dst, !op1);
    }

    // =========================================================================
    // Helper functions for memory operations
    // =========================================================================

    // resolve_addr32 is defined in logical8.rs to avoid duplicate definitions

    // get_laddr32_seg is defined in logical8.rs to avoid duplicate definitions

    /// Read word from virtual address
    pub fn read_virtual_word(&self, seg: BxSegregs, eaddr: u32) -> u16 {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        self.mem_read_word(laddr as u64)
    }

    /// Read-Modify-Write: Read word, return it and linear address for write back
    pub fn read_rmw_virtual_word(&mut self, seg: BxSegregs, eaddr: u32) -> (u16, u32) {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        let val = self.mem_read_word(laddr as u64);
        (val, laddr)
    }

    /// Write word to linear address (for RMW operations)
    pub fn write_rmw_linear_word(&mut self, laddr: u32, val: u16) {
        self.mem_write_word(laddr as u64, val);
    }

    // =========================================================================
    // Memory-form instructions
    // =========================================================================

    /// XOR_EwGwM: XOR r/m16, r16 (memory form)
    /// Matches BX_CPU_C::XOR_EwGwM
    pub fn xor_ew_gw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 ^ op2_16;

        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "XOR16 mem: [{:?}:{:#x}] = {:#06x} ^ {:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            op2_16,
            result
        );
    }

    /// XOR_GwEwM: XOR r16, r/m16 (memory form)
    /// Matches BX_CPU_C::XOR_GwEwM
    pub fn xor_gw_ew_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_16 = self.get_gpr16(dst_reg);
        let result = op1_16 ^ op2_16;

        self.set_gpr16(dst_reg, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "XOR16 mem: reg{} = {:#06x} ^ {:#06x} = {:#06x}",
            dst_reg,
            op1_16,
            op2_16,
            result
        );
    }

    /// XOR_EwIwM: XOR r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::XOR_EwIwM
    pub fn xor_ew_iw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let op2_16 = instr.iw();
        let result = op1_16 ^ op2_16;

        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "XOR16 mem: [{:?}:{:#x}] = {:#06x} ^ {:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            op2_16,
            result
        );
    }

    /// OR_EwGwM: OR r/m16, r16 (memory form)
    /// Matches BX_CPU_C::OR_EwGwM
    pub fn or_ew_gw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 | op2_16;

        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "OR16 mem: [{:?}:{:#x}] = {:#06x} | {:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            op2_16,
            result
        );
    }

    /// OR_GwEwM: OR r16, r/m16 (memory form)
    /// Matches BX_CPU_C::OR_GwEwM
    pub fn or_gw_ew_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_16 = self.get_gpr16(dst_reg);
        let result = op1_16 | op2_16;

        self.set_gpr16(dst_reg, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "OR16 mem: reg{} = {:#06x} | {:#06x} = {:#06x}",
            dst_reg,
            op1_16,
            op2_16,
            result
        );
    }

    /// OR_EwIwM: OR r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::OR_EwIwM
    pub fn or_ew_iw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let op2_16 = instr.iw();
        let result = op1_16 | op2_16;

        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "OR16 mem: [{:?}:{:#x}] = {:#06x} | {:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            op2_16,
            result
        );
    }

    /// AND_EwGwM: AND r/m16, r16 (memory form)
    /// Matches BX_CPU_C::AND_EwGwM
    pub fn and_ew_gw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 & op2_16;

        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "AND16 mem: [{:?}:{:#x}] = {:#06x} & {:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            op2_16,
            result
        );
    }

    /// AND_GwEwM: AND r16, r/m16 (memory form)
    /// Matches BX_CPU_C::AND_GwEwM
    pub fn and_gw_ew_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_16 = self.get_gpr16(dst_reg);
        let result = op1_16 & op2_16;

        self.set_gpr16(dst_reg, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "AND16 mem: reg{} = {:#06x} & {:#06x} = {:#06x}",
            dst_reg,
            op1_16,
            op2_16,
            result
        );
    }

    /// AND_EwIwM: AND r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::AND_EwIwM
    pub fn and_ew_iw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let op2_16 = instr.iw();
        let result = op1_16 & op2_16;

        self.write_rmw_linear_word(laddr, result);
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "AND16 mem: [{:?}:{:#x}] = {:#06x} & {:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            op2_16,
            result
        );
    }

    /// NOT_EwM: NOT r/m16 (memory form)
    /// Matches BX_CPU_C::NOT_EwM
    pub fn not_ew_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_16, laddr) = self.read_rmw_virtual_word(seg, eaddr);
        let result = !op1_16;

        self.write_rmw_linear_word(laddr, result);
        tracing::trace!(
            "NOT16 mem: [{:?}:{:#x}] = !{:#06x} = {:#06x}",
            seg,
            eaddr,
            op1_16,
            result
        );
    }

    /// TEST_EwGwM: TEST r/m16, r16 (memory form)
    /// Matches BX_CPU_C::TEST_EwGwM
    pub fn test_ew_gw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.read_virtual_word(seg, eaddr);
        let src_reg = instr.src() as usize; // src()=[1]=nnn=register for 16-bit store (decoder swaps)
        let op2_16 = self.get_gpr16(src_reg);
        let result = op1_16 & op2_16;

        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "TEST16 mem: [{:?}:{:#x}] & reg{} = {:#06x} & {:#06x} = {:#06x}",
            seg,
            eaddr,
            src_reg,
            op1_16,
            op2_16,
            result
        );
    }

    /// TEST_EwIwM: TEST r/m16, imm16 (memory form)
    /// Matches BX_CPU_C::TEST_EwIwM
    pub fn test_ew_iw_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_16 = self.read_virtual_word(seg, eaddr);
        let op2_16 = instr.iw();
        let result = op1_16 & op2_16;

        self.set_flags_oszapc_logic_16(result);
        tracing::trace!(
            "TEST16 mem: [{:?}:{:#x}] & {:#06x} = {:#06x} & {:#06x} = {:#06x}",
            seg,
            eaddr,
            op2_16,
            op1_16,
            op2_16,
            result
        );
    }

    // =========================================================================
    // Unified handlers: dispatch R/M based on instr.mod_c0()
    // =========================================================================

    pub fn xor_ew_gw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_ew_gw_r(instr) } else { self.xor_ew_gw_m(instr) }
    }
    pub fn xor_gw_ew(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_gw_ew_r(instr) } else { self.xor_gw_ew_m(instr) }
    }
    pub fn xor_ew_iw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_ew_iw_r(instr) } else { self.xor_ew_iw_m(instr) }
    }
    pub fn and_ew_gw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_ew_gw_r(instr) } else { self.and_ew_gw_m(instr) }
    }
    pub fn and_gw_ew(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_gw_ew_r(instr) } else { self.and_gw_ew_m(instr) }
    }
    pub fn and_ew_iw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_ew_iw_r(instr) } else { self.and_ew_iw_m(instr) }
    }
    pub fn or_ew_gw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_ew_gw_r(instr) } else { self.or_ew_gw_m(instr) }
    }
    pub fn or_gw_ew(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_gw_ew_r(instr) } else { self.or_gw_ew_m(instr) }
    }
    pub fn or_ew_iw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_ew_iw_r(instr) } else { self.or_ew_iw_m(instr) }
    }
    pub fn not_ew(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.not_ew_r(instr) } else { self.not_ew_m(instr) }
    }
    pub fn test_ew_gw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.test_ew_gw_r(instr) } else { self.test_ew_gw_m(instr) }
    }
    pub fn test_ew_iw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.test_ew_iw_r(instr) } else { self.test_ew_iw_m(instr) }
    }
    pub fn cmp_gw_ew(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_gw_ew_r(instr) } else { self.cmp_gw_ew_m(instr) }
    }
    pub fn cmp_ew_iw(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_ew_iw_r(instr) } else { self.cmp_ew_iw_m(instr) }
    }
}
