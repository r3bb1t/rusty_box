//! 32-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical32.cc

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 32-bit logical operations
    pub fn set_flags_oszapc_logic_32(&mut self, result: u32) {
        let sf = (result & 0x80000000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if pf { self.eflags |= 1 << 2; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
    }

    /// Update flags for 32-bit subtraction
    pub fn set_flags_oszapc_sub_32(&mut self, op1: u32, op2: u32, result: u32) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x80000000) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
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
    // ZERO_IDIOM - XOR register with itself (optimization: set to 0)
    // =========================================================================

    /// ZERO_IDIOM_GdR: XOR r32, r32 (zero idiom - set register to 0)
    /// Matches BX_CPU_C::ZERO_IDIOM_GdR
    pub fn zero_idiom_gd_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        self.set_gpr32(dst, 0);
        self.set_flags_oszapc_logic_32(0);
    }

    // =========================================================================
    // CMP instructions
    // =========================================================================

    /// CMP r32, r32
    pub fn cmp_gd_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        tracing::trace!("CMP r32, r32: {:#010x} - {:#010x} = {:#010x}", op1, op2, result);
    }

    /// CMP EAX, imm32
    pub fn cmp_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0); // EAX
        let op2 = instr.id();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        tracing::trace!("CMP EAX, imm32: {:#010x} - {:#010x}", op1, op2);
    }

    /// CMP r/m32, imm32
    pub fn cmp_ed_id_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        tracing::trace!("CMP r32, imm32: {:#010x} - {:#010x}", op1, op2);
    }

    // =========================================================================
    // TEST instructions
    // =========================================================================

    /// TEST r32, r32
    pub fn test_ed_gd_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST r32, r32: {:#010x} & {:#010x} = {:#010x}", op1, op2, result);
    }

    /// TEST EAX, imm32
    pub fn test_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0); // EAX
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST EAX, imm32: {:#010x} & {:#010x}", op1, op2);
    }

    /// TEST r/m32, imm32
    pub fn test_ed_id_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST r32, imm32: {:#010x} & {:#010x}", op1, op2);
    }

    // =========================================================================
    // AND instructions
    // =========================================================================

    /// AND r32, r32
    pub fn and_gd_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 & op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// AND EAX, imm32
    pub fn and_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_gpr32(0, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// AND r/m32, imm32
    pub fn and_ed_id_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    // =========================================================================
    // XOR instructions
    // =========================================================================

    /// XOR_GdEdR: XOR r32, r32 (register form)
    /// Matches BX_CPU_C::XOR_GdEdR
    pub fn xor_gd_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 ^ op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// XOR_EdIdR: XOR r/m32, imm32 (register form)
    /// Matches BX_CPU_C::XOR_EdIdR
    pub fn xor_ed_id_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 ^ op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    // =========================================================================
    // OR instructions
    // =========================================================================

    /// OR r32, r32
    pub fn or_gd_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1 | op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// OR EAX, imm32
    pub fn or_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0);
        let op2 = instr.id();
        let result = op1 | op2;
        self.set_gpr32(0, result);
        self.set_flags_oszapc_logic_32(result);
    }

    /// OR_EdIdR: OR r/m32, imm32 (register form)
    /// Matches BX_CPU_C::OR_EdIdR
    pub fn or_ed_id_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = instr.id();
        let result = op1 | op2;
        self.set_gpr32(dst, result);
        self.set_flags_oszapc_logic_32(result);
    }

    // =========================================================================
    // NOT instructions
    // =========================================================================

    /// NOT r32
    pub fn not_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        self.set_gpr32(dst, !op1);
    }

    // =========================================================================
    // INC/DEC instructions
    // =========================================================================

    /// INC r32
    pub fn inc_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let result = op1.wrapping_add(1);
        self.set_gpr32(dst, result);
        
        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = result == 0x80000000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
        
        tracing::trace!("INC r32: {:#010x} + 1 = {:#010x}", op1, result);
    }

    /// DEC r32
    pub fn dec_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        let result = op1.wrapping_sub(1);
        self.set_gpr32(dst, result);
        
        let zf = result == 0;
        let sf = (result & 0x80000000) != 0;
        let of = op1 == 0x80000000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
        
        tracing::trace!("DEC r32: {:#010x} - 1 = {:#010x}", op1, result);
    }

    // =========================================================================
    // Helper functions for memory operations
    // =========================================================================

    // resolve_addr32 is defined in logical8.rs to avoid duplicate definitions

    // get_laddr32_seg is defined in logical8.rs to avoid duplicate definitions

    /// Read dword from virtual address
    pub fn read_virtual_dword(&self, seg: BxSegregs, eaddr: u32) -> u32 {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        self.mem_read_dword(laddr as u64)
    }

    /// Read-Modify-Write: Read dword, return it and linear address for write back
    pub fn read_rmw_virtual_dword(&mut self, seg: BxSegregs, eaddr: u32) -> (u32, u32) {
        let laddr = self.get_laddr32_seg(seg, eaddr);
        let val = self.mem_read_dword(laddr as u64);
        (val, laddr)
    }

    /// Write dword to linear address (for RMW operations)
    pub fn write_rmw_linear_dword(&mut self, laddr: u32, val: u32) {
        self.mem_write_dword(laddr as u64, val);
    }

    // =========================================================================
    // Memory-form instructions
    // =========================================================================

    /// XOR_EdGdM: XOR r/m32, r32 (memory form)
    /// Matches BX_CPU_C::XOR_EdGdM
    pub fn xor_ed_gd_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 ^ op2_32;

        self.write_rmw_linear_dword(laddr, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("XOR32 mem: [{:?}:{:#x}] = {:#010x} ^ {:#010x} = {:#010x}", seg, eaddr, op1_32, op2_32, result);
    }

    /// XOR_GdEdM: XOR r32, r/m32 (memory form)
    /// Matches BX_CPU_C::XOR_GdEdM
    pub fn xor_gd_ed_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.read_virtual_dword(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32 ^ op2_32;

        self.set_gpr32(dst_reg, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("XOR32 mem: reg{} = {:#010x} ^ {:#010x} = {:#010x}", dst_reg, op1_32, op2_32, result);
    }

    /// XOR_EdIdM: XOR r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::XOR_EdIdM
    pub fn xor_ed_id_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let op2_32 = instr.id();
        let result = op1_32 ^ op2_32;

        self.write_rmw_linear_dword(laddr, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("XOR32 mem: [{:?}:{:#x}] = {:#010x} ^ {:#010x} = {:#010x}", seg, eaddr, op1_32, op2_32, result);
    }

    /// OR_EdGdM: OR r/m32, r32 (memory form)
    /// Matches BX_CPU_C::OR_EdGdM
    pub fn or_ed_gd_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 | op2_32;

        self.write_rmw_linear_dword(laddr, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("OR32 mem: [{:?}:{:#x}] = {:#010x} | {:#010x} = {:#010x}", seg, eaddr, op1_32, op2_32, result);
    }

    /// OR_GdEdM: OR r32, r/m32 (memory form)
    /// Matches BX_CPU_C::OR_GdEdM
    pub fn or_gd_ed_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.read_virtual_dword(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32 | op2_32;

        self.set_gpr32(dst_reg, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("OR32 mem: reg{} = {:#010x} | {:#010x} = {:#010x}", dst_reg, op1_32, op2_32, result);
    }

    /// OR_EdIdM: OR r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::OR_EdIdM
    pub fn or_ed_id_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let op2_32 = instr.id();
        let result = op1_32 | op2_32;

        self.write_rmw_linear_dword(laddr, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("OR32 mem: [{:?}:{:#x}] = {:#010x} | {:#010x} = {:#010x}", seg, eaddr, op1_32, op2_32, result);
    }

    /// AND_EdGdM: AND r/m32, r32 (memory form)
    /// Matches BX_CPU_C::AND_EdGdM
    pub fn and_ed_gd_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 & op2_32;

        self.write_rmw_linear_dword(laddr, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("AND32 mem: [{:?}:{:#x}] = {:#010x} & {:#010x} = {:#010x}", seg, eaddr, op1_32, op2_32, result);
    }

    /// AND_GdEdM: AND r32, r/m32 (memory form)
    /// Matches BX_CPU_C::AND_GdEdM
    pub fn and_gd_ed_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.read_virtual_dword(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32 & op2_32;

        self.set_gpr32(dst_reg, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("AND32 mem: reg{} = {:#010x} & {:#010x} = {:#010x}", dst_reg, op1_32, op2_32, result);
    }

    /// AND_EdIdM: AND r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::AND_EdIdM
    pub fn and_ed_id_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let op2_32 = instr.id();
        let result = op1_32 & op2_32;

        self.write_rmw_linear_dword(laddr, result);
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("AND32 mem: [{:?}:{:#x}] = {:#010x} & {:#010x} = {:#010x}", seg, eaddr, op1_32, op2_32, result);
    }

    /// NOT_EdM: NOT r/m32 (memory form)
    /// Matches BX_CPU_C::NOT_EdM
    pub fn not_ed_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let result = !op1_32;

        self.write_rmw_linear_dword(laddr, result);
        tracing::trace!("NOT32 mem: [{:?}:{:#x}] = !{:#010x} = {:#010x}", seg, eaddr, op1_32, result);
    }

    /// TEST_EdGdM: TEST r/m32, r32 (memory form)
    /// Matches BX_CPU_C::TEST_EdGdM
    pub fn test_ed_gd_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.read_virtual_dword(seg, eaddr);
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32 & op2_32;

        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST32 mem: [{:?}:{:#x}] & reg{} = {:#010x} & {:#010x} = {:#010x}", seg, eaddr, src_reg, op1_32, op2_32, result);
    }

    /// TEST_EdIdM: TEST r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::TEST_EdIdM
    pub fn test_ed_id_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.read_virtual_dword(seg, eaddr);
        let op2_32 = instr.id();
        let result = op1_32 & op2_32;

        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST32 mem: [{:?}:{:#x}] & {:#010x} = {:#010x} & {:#010x} = {:#010x}", seg, eaddr, op2_32, op1_32, op2_32, result);
    }

    /// CMP_GdEdM: CMP r32, r/m32 (memory form)
    /// Matches BX_CPU_C::CMP_GdEdM
    pub fn cmp_gd_ed_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_32 = self.read_virtual_dword(seg, eaddr);
        let dst_reg = instr.dst() as usize;
        let op1_32 = self.get_gpr32(dst_reg);
        let result = op1_32.wrapping_sub(op2_32);
        self.set_flags_oszapc_sub_32(op1_32, op2_32, result);
    }

    /// CMP_EdGdR: CMP r/m32, r32 (register form)
    /// Matches BX_CPU_C::CMP_EdGdR
    pub fn cmp_ed_gd_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr32(dst);
        let op2 = self.get_gpr32(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
    }

    /// CMP_EdGdM: CMP r/m32, r32 (memory form)
    /// Matches BX_CPU_C::CMP_EdGdM
    pub fn cmp_ed_gd_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let (op1_32, _laddr) = self.read_rmw_virtual_dword(seg, eaddr);
        let src_reg = instr.src() as usize;
        let op2_32 = self.get_gpr32(src_reg);
        let result = op1_32.wrapping_sub(op2_32);
        self.set_flags_oszapc_sub_32(op1_32, op2_32, result);
    }

    /// CMP_EdIdM: CMP r/m32, imm32 (memory form)
    /// Matches BX_CPU_C::CMP_EdIdM
    pub fn cmp_ed_id_m(&mut self, instr: &BxInstructionGenerated) {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1_32 = self.read_virtual_dword(seg, eaddr);
        let op2_32 = instr.id();
        let result = op1_32.wrapping_sub(op2_32);
        self.set_flags_oszapc_sub_32(op1_32, op2_32, result);
    }

    // =========================================================================
    // Unified handlers: dispatch R/M based on instr.mod_c0()
    // =========================================================================

    /// XOR r/m32, r32 - unified (EdGd: memory is read-modify-write, register is commutative)
    pub fn xor_ed_gd(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_gd_ed_r(instr) } else { self.xor_ed_gd_m(instr) }
    }

    /// XOR r32, r/m32 - unified (GdEd: register dest, memory is read-only source)
    pub fn xor_gd_ed(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_gd_ed_r(instr) } else { self.xor_gd_ed_m(instr) }
    }

    /// XOR r/m32, imm32 - unified
    pub fn xor_ed_id(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.xor_ed_id_r(instr) } else { self.xor_ed_id_m(instr) }
    }

    /// AND r/m32, r32 - unified (EdGd: memory is read-modify-write, register is commutative)
    pub fn and_ed_gd(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_gd_ed_r(instr) } else { self.and_ed_gd_m(instr) }
    }

    /// AND r32, r/m32 - unified (GdEd: register dest, memory is read-only source)
    pub fn and_gd_ed(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_gd_ed_r(instr) } else { self.and_gd_ed_m(instr) }
    }

    /// AND r/m32, imm32 - unified (handles both AndEdId and AndEdsIb opcodes)
    pub fn and_ed_id(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.and_ed_id_r(instr) } else { self.and_ed_id_m(instr) }
    }

    /// OR r/m32, r32 - unified (EdGd: memory is read-modify-write, register is commutative)
    pub fn or_ed_gd(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_gd_ed_r(instr) } else { self.or_ed_gd_m(instr) }
    }

    /// OR r32, r/m32 - unified (GdEd: register dest, memory is read-only source)
    pub fn or_gd_ed(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_gd_ed_r(instr) } else { self.or_gd_ed_m(instr) }
    }

    /// OR r/m32, imm32 - unified
    pub fn or_ed_id(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.or_ed_id_r(instr) } else { self.or_ed_id_m(instr) }
    }

    /// NOT r/m32 - unified
    pub fn not_ed(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.not_ed_r(instr) } else { self.not_ed_m(instr) }
    }

    /// TEST r/m32, r32 - unified
    pub fn test_ed_gd(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.test_ed_gd_r(instr) } else { self.test_ed_gd_m(instr) }
    }

    /// TEST r/m32, imm32 - unified
    pub fn test_ed_id(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.test_ed_id_r(instr) } else { self.test_ed_id_m(instr) }
    }

    /// CMP r32, r/m32 - unified (GdEd: register dest compares with reg or memory)
    pub fn cmp_gd_ed(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_gd_ed_r(instr) } else { self.cmp_gd_ed_m(instr) }
    }

    /// CMP r/m32, r32 - unified (EdGd: memory or register compared with register)
    pub fn cmp_ed_gd(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_ed_gd_r(instr) } else { self.cmp_ed_gd_m(instr) }
    }

    /// CMP r/m32, imm32 - unified (handles both CmpEdId and CmpEdsIb opcodes)
    pub fn cmp_ed_id(&mut self, instr: &BxInstructionGenerated) {
        if instr.mod_c0() { self.cmp_ed_id_r(instr) } else { self.cmp_ed_id_m(instr) }
    }
}
