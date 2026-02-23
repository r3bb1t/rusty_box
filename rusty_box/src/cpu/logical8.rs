//! 8-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical8.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
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
        let pf = result.count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if pf { self.eflags |= 1 << 2; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        // OF=0, CF=0 are already cleared
    }

    /// Update flags for 8-bit subtraction (CMP, SUB)
    pub fn set_flags_oszapc_sub_8(&mut self, op1: u8, op2: u8, result: u8) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x80) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x80) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = result.count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if cf { self.eflags |= 1 << 0; }
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    // =========================================================================
    // XOR instructions
    // =========================================================================

    /// XOR_EbIbR: XOR r/m8, imm8 (register form)
    /// Opcode: 0x80/6 (8-bit)
    /// Matches BX_CPU_C::XOR_EbIbR
    pub fn xor_eb_ib_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst, extend8bit_l);
        let op2 = instr.ib();
        let result = op1 ^ op2;

        self.write_8bit_regx(dst, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// XOR_GbEbR: XOR r8, r/m8 (register form)
    /// Matches BX_CPU_C::XOR_GbEbR
    pub fn xor_gb_eb_r(&mut self, instr: &BxInstructionGenerated) {
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
    /// Opcode 0x30: reg (meta_data[0]) = SOURCE, rm (meta_data[1]) = DESTINATION
    pub fn xor_eb_gb_r(&mut self, instr: &BxInstructionGenerated) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.meta_data[1] as usize, extend8bit_l);  // rm = destination
        let op2 = self.read_8bit_regx(instr.meta_data[0] as usize, extend8bit_l);  // reg = source
        let result = op1 ^ op2;
        self.write_8bit_regx(instr.meta_data[1] as usize, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    // =========================================================================
    // Helper functions
    // =========================================================================

    /// Read 8-bit register with extend8bitL support (matches BX_READ_8BIT_REGx)
    pub fn read_8bit_regx(&self, reg_idx: usize, extend8bit_l: u8) -> u8 {
        if extend8bit_l != 0 || (reg_idx & 4) == 0 {
            self.get_gpr8(reg_idx)
        } else {
            let reg16_idx = reg_idx & 0x3;
            (self.get_gpr16(reg16_idx) >> 8) as u8
        }
    }

    /// Write 8-bit register with extend8bitL support (matches BX_WRITE_8BIT_REGx)
    pub fn write_8bit_regx(&mut self, reg_idx: usize, extend8bit_l: u8, val: u8) {
        if extend8bit_l != 0 || (reg_idx & 4) == 0 {
            self.set_gpr8(reg_idx, val);
        } else {
            let reg16_idx = reg_idx & 0x3;
            let current = self.get_gpr16(reg16_idx);
            let new_val = (current & 0x00FF) | ((val as u16) << 8);
            self.set_gpr16(reg16_idx, new_val);
        }
    }

    /// Resolve effective address (matches BX_CPU_RESOLVE_ADDR)
    pub fn resolve_addr32(&self, instr: &BxInstructionGenerated) -> u32 {
        let base_reg = instr.sib_base() as usize;
        let mut eaddr = if base_reg < 16 {
            self.get_gpr32(base_reg)
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

    /// Get linear address from segment and offset
    pub fn get_laddr32_seg(&self, seg: BxSegregs, offset: u32) -> u32 {
        let seg_base = self.get_segment_base(seg);
        (seg_base.wrapping_add(offset as u64)) as u32
    }

    /// Read-Modify-Write: Read byte, return it and linear address for write back
    pub fn read_rmw_virtual_byte(&mut self, seg: BxSegregs, eaddr: u32) -> (u8, u32) {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        let val = self.mem_read_byte(laddr as u64);
        (val, laddr)
    }

    /// Write byte to linear address (for RMW operations)
    pub fn write_rmw_linear_byte(&mut self, laddr: u32, val: u8) {
        self.mem_write_byte(laddr as u64, val);
    }

    /// Read byte from virtual address
    pub fn read_virtual_byte(&self, seg: BxSegregs, eaddr: u32) -> u8 {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        self.mem_read_byte(laddr as u64)
    }

    // =========================================================================
    // CMP instructions
    // =========================================================================

    /// CMP r8, r8
    pub fn cmp_gb_eb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr8(dst);
        let op2 = self.get_gpr8(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
        tracing::trace!("CMP r8, r8: {:#04x} - {:#04x} = {:#04x}", op1, op2, result);
    }

    /// CMP AL, imm8
    pub fn cmp_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
        tracing::trace!("CMP AL, imm8: {:#04x} - {:#04x}", op1, op2);
    }

    /// CMP_GbEb_M: CMP r8, r/m8 (memory form)
    pub fn cmp_gb_eb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l());
        let op2 = self.read_virtual_byte(seg, eaddr);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    /// CMP_EbGb_M: CMP r/m8, r8 (memory form)
    pub fn cmp_eb_gb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_virtual_byte(seg, eaddr);
        let op2 = self.read_8bit_regx(instr.dst() as usize, instr.extend8bit_l()); // reg field = source
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    /// CMP_EbIb_M: CMP r/m8, imm8 (memory form)
    pub fn cmp_eb_ib_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_virtual_byte(seg, eaddr);
        let op2 = instr.ib();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST r8, r8
    pub fn test_eb_gb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr8(dst);
        let op2 = self.get_gpr8(src);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("TEST r8, r8: {:#04x} & {:#04x} = {:#04x}", op1, op2, result);
    }

    /// TEST AL, imm8
    pub fn test_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("TEST AL, imm8: {:#04x} & {:#04x}", op1, op2);
    }

    /// TEST_EbIbR: TEST r8, imm8 (register form)
    /// Matches BX_CPU_C::TEST_EbIbR
    pub fn test_eb_ib_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn and_gb_eb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr8(dst);
        let op2 = self.get_gpr8(src);
        let result = op1 & op2;
        self.set_gpr8(dst, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("AND r8, r8: {:#04x} & {:#04x} = {:#04x}", op1, op2, result);
    }

    /// AND_EbGbR: AND r/m8, r8 (register form, store-direction)
    /// Opcode 0x20: reg (meta_data[0]) = SOURCE, rm (meta_data[1]) = DESTINATION
    pub fn and_eb_gb_r(&mut self, instr: &BxInstructionGenerated) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.meta_data[1] as usize, extend8bit_l);  // rm = destination
        let op2 = self.read_8bit_regx(instr.meta_data[0] as usize, extend8bit_l);  // reg = source
        let result = op1 & op2;
        self.write_8bit_regx(instr.meta_data[1] as usize, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// AND AL, imm8
    pub fn and_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0);
        let op2 = instr.ib();
        let result = op1 & op2;
        self.set_gpr8(0, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// AND_EbIbR: AND r8, imm8 (register form)
    /// Matches BX_CPU_C::AND_EbIbR
    pub fn and_eb_ib_r(&mut self, instr: &BxInstructionGenerated) {
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
    /// Opcode 0x08: reg (meta_data[0]) = SOURCE, rm (meta_data[1]) = DESTINATION
    pub fn or_eb_gb_r(&mut self, instr: &BxInstructionGenerated) {
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(instr.meta_data[1] as usize, extend8bit_l);  // rm = destination
        let op2 = self.read_8bit_regx(instr.meta_data[0] as usize, extend8bit_l);  // reg = source
        let result = op1 | op2;
        self.write_8bit_regx(instr.meta_data[1] as usize, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR r8, r8 (load-direction, opcode 0x0A)
    pub fn or_gb_eb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr8(dst);
        let op2 = self.get_gpr8(src);
        let result = op1 | op2;
        self.set_gpr8(dst, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR AL, imm8
    pub fn or_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0);
        let op2 = instr.ib();
        let result = op1 | op2;
        self.set_gpr8(0, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR_EbIbR: OR r8, imm8 (register form)
    /// Matches BX_CPU_C::OR_EbIbR
    pub fn or_eb_ib_r(&mut self, instr: &BxInstructionGenerated) {
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
    pub fn not_eb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr8(dst);
        self.set_gpr8(dst, !op1);
        // NOT does not affect flags
    }

    /// NOT r/m8 (memory form)
    /// Matches BX_CPU_C::NOT_EbM
    pub fn not_eb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_8, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let result = !op1_8;

        self.write_rmw_linear_byte(laddr, result);
        tracing::trace!("NOT8 mem: [{:?}:{:#x}] = !{:#04x} = {:#04x}", seg, eaddr, op1_8, result);
    }

    // =========================================================================
    // Memory-form instructions
    // =========================================================================

    /// XOR_EbGbM: XOR r/m8, r8 (memory form)
    /// Matches BX_CPU_C::XOR_EbGbM
    pub fn xor_eb_gb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 ^ op2;

        self.write_rmw_linear_byte(laddr, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("XOR8 mem: [{:?}:{:#x}] = {:#04x} ^ {:#04x} = {:#04x}", seg, eaddr, op1, op2, result);
    }

    /// XOR_GbEbM: XOR r8, r/m8 (memory form)
    /// Matches BX_CPU_C::XOR_GbEbM
    pub fn xor_gb_eb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst_reg, extend8bit_l);
        let result = op1 ^ op2;

        self.write_8bit_regx(dst_reg, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("XOR8 mem: reg{} = {:#04x} ^ {:#04x} = {:#04x}", dst_reg, op1, op2, result);
    }

    /// XOR_EbIbM: XOR r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::XOR_EbIbM
    pub fn xor_eb_ib_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let op2 = instr.ib();
        let result = op1 ^ op2;

        self.write_rmw_linear_byte(laddr, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("XOR8 mem: [{:?}:{:#x}] = {:#04x} ^ {:#04x} = {:#04x}", seg, eaddr, op1, op2, result);
    }

    /// OR_EbGbM: OR r/m8, r8 (memory form)
    /// Matches BX_CPU_C::OR_EbGbM
    pub fn or_eb_gb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 | op2;

        self.write_rmw_linear_byte(laddr, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("OR8 mem: [{:?}:{:#x}] = {:#04x} | {:#04x} = {:#04x}", seg, eaddr, op1, op2, result);
    }

    /// OR_GbEbM: OR r8, r/m8 (memory form)
    /// Matches BX_CPU_C::OR_GbEbM
    pub fn or_gb_eb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst_reg, extend8bit_l);
        let result = op1 | op2;

        self.write_8bit_regx(dst_reg, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("OR8 mem: reg{} = {:#04x} | {:#04x} = {:#04x}", dst_reg, op1, op2, result);
    }

    /// OR_EbIbM: OR r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::OR_EbIbM
    pub fn or_eb_ib_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let op2 = instr.ib();
        let result = op1 | op2;

        self.write_rmw_linear_byte(laddr, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("OR8 mem: [{:?}:{:#x}] = {:#04x} | {:#04x} = {:#04x}", seg, eaddr, op1, op2, result);
    }

    /// AND_EbGbM: AND r/m8, r8 (memory form)
    /// Matches BX_CPU_C::AND_EbGbM
    pub fn and_eb_gb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 & op2;

        self.write_rmw_linear_byte(laddr, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("AND8 mem: [{:?}:{:#x}] = {:#04x} & {:#04x} = {:#04x}", seg, eaddr, op1, op2, result);
    }

    /// AND_GbEbM: AND r8, r/m8 (memory form)
    /// Matches BX_CPU_C::AND_GbEbM
    pub fn and_gb_eb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2 = self.read_virtual_byte(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op1 = self.read_8bit_regx(dst_reg, extend8bit_l);
        let result = op1 & op2;

        self.write_8bit_regx(dst_reg, extend8bit_l, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("AND8 mem: reg{} = {:#04x} & {:#04x} = {:#04x}", dst_reg, op1, op2, result);
    }

    /// AND_EbIbM: AND r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::AND_EbIbM
    pub fn and_eb_ib_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1, laddr) = self.read_rmw_virtual_byte(seg, eaddr);
        let op2 = instr.ib();
        let result = op1 & op2;

        self.write_rmw_linear_byte(laddr, result);
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("AND8 mem: [{:?}:{:#x}] = {:#04x} & {:#04x} = {:#04x}", seg, eaddr, op1, op2, result);
    }

    /// TEST_EbGbM: TEST r/m8, r8 (memory form)
    /// Matches BX_CPU_C::TEST_EbGbM
    pub fn test_eb_gb_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_virtual_byte(seg, eaddr);
        let src_reg = instr.dst() as usize; // reg field = source for store-direction
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);
        let result = op1 & op2;

        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("TEST8 mem: [{:?}:{:#x}] & reg{} = {:#04x} & {:#04x} = {:#04x}", seg, eaddr, src_reg, op1, op2, result);
    }

    /// TEST_EbIbM: TEST r/m8, imm8 (memory form)
    /// Matches BX_CPU_C::TEST_EbIbM
    pub fn test_eb_ib_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_virtual_byte(seg, eaddr);
        let op2 = instr.ib();
        let result = op1 & op2;

        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("TEST8 mem: [{:?}:{:#x}] & {:#04x} = {:#04x} & {:#04x} = {:#04x}", seg, eaddr, op2, op1, op2, result);
    }

    // =========================================================================
    // CMP register-form instructions (needed for unified dispatchers)
    // =========================================================================

    /// CMP_EbGbR: CMP r/m8, r8 (register form)
    /// Opcode 0x38: reg (dst()) = second operand, rm (src()) = first operand
    pub fn cmp_eb_gb_r(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(instr.src() as usize);  // rm = first operand
        let op2 = self.get_gpr8(instr.dst() as usize);  // reg = second operand
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    /// CMP_EbIbR: CMP r/m8, imm8 (register form)
    /// Matches BX_CPU_C::CMP_EbIbR
    pub fn cmp_eb_ib_r(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(instr.dst() as usize);
        let op2 = instr.ib();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
    }

    // =========================================================================
    // Unified handlers: dispatch R/M based on instr.mod_c0()
    // =========================================================================

    pub fn and_eb_gb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_eb_gb_r(instr) } else { self.and_eb_gb_m(instr) }
    }
    pub fn and_gb_eb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_gb_eb_r(instr) } else { self.and_gb_eb_m(instr) }
    }
    pub fn and_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_eb_ib_r(instr) } else { self.and_eb_ib_m(instr) }
    }
    pub fn or_eb_gb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_eb_gb_r(instr) } else { self.or_eb_gb_m(instr) }
    }
    pub fn or_gb_eb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_gb_eb_r(instr) } else { self.or_gb_eb_m(instr) }
    }
    pub fn or_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_eb_ib_r(instr) } else { self.or_eb_ib_m(instr) }
    }
    pub fn xor_eb_gb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_eb_gb_r(instr) } else { self.xor_eb_gb_m(instr) }
    }
    pub fn xor_gb_eb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_gb_eb_r(instr) } else { self.xor_gb_eb_m(instr) }
    }
    pub fn xor_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_eb_ib_r(instr) } else { self.xor_eb_ib_m(instr) }
    }
    pub fn not_eb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.not_eb_r(instr) } else { self.not_eb_m(instr) }
    }
    pub fn test_eb_gb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.test_eb_gb_r(instr) } else { self.test_eb_gb_m(instr) }
    }
    pub fn test_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.test_eb_ib_r(instr) } else { self.test_eb_ib_m(instr) }
    }
    pub fn cmp_gb_eb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_gb_eb_r(instr) } else { self.cmp_gb_eb_m(instr) }
    }
    pub fn cmp_eb_gb(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_eb_gb_r(instr) } else { self.cmp_eb_gb_m(instr) }
    }
    pub fn cmp_eb_ib(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_eb_ib_r(instr) } else { self.cmp_eb_ib_m(instr) }
    }
}
