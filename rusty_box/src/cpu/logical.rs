//! Logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical16.cc, logical32.cc, arith16.cc, arith32.cc
//! Copyright (C) 2001-2019 The Bochs Project

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::BxInstructionGenerated,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 8-bit logical operations (AND, OR, XOR, TEST)
    fn set_flags_oszapc_logic_8(&mut self, result: u8) {
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

    /// Update flags for 16-bit logical operations
    fn set_flags_oszapc_logic_16(&mut self, result: u16) {
        let sf = (result & 0x8000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if pf { self.eflags |= 1 << 2; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
    }

    /// Update flags for 32-bit logical operations
    fn set_flags_oszapc_logic_32(&mut self, result: u32) {
        let sf = (result & 0x80000000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if pf { self.eflags |= 1 << 2; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
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

    /// XOR_EbIbR: XOR r/m8, imm8 (register form)
    /// Opcode: 0x80/6 or 0x83/6 (8-bit)
    /// Matches BX_CPU_C::XOR_EbIbR
    /// This covers XOR AL, imm8 (XorAlib opcode)
    pub fn xor_eb_ib_r(&mut self, instr: &BxInstructionGenerated) {
        // For XOR AL, imm8, we can use AL directly
        let op1 = self.al();
        let op2 = instr.ib();
        let result = op1 ^ op2;
        
        self.set_al(result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// Update flags for 8-bit subtraction (CMP, SUB)
    fn set_flags_oszapc_sub_8(&mut self, op1: u8, op2: u8, result: u8) {
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

    /// Update flags for 16-bit subtraction
    fn set_flags_oszapc_sub_16(&mut self, op1: u16, op2: u16, result: u16) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x8000) != 0;
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

    /// CMP r16, r16
    pub fn cmp_gw_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        tracing::trace!("CMP r16, r16: {:#06x} - {:#06x} = {:#06x}", op1, op2, result);
    }

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

    /// CMP AL, imm8
    pub fn cmp_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_8(op1, op2, result);
        tracing::trace!("CMP AL, imm8: {:#04x} - {:#04x}", op1, op2);
    }

    /// CMP AX, imm16
    pub fn cmp_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_16(op1, op2, result);
        tracing::trace!("CMP AX, imm16: {:#06x} - {:#06x}", op1, op2);
    }

    /// CMP EAX, imm32
    pub fn cmp_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0); // EAX
        let op2 = instr.id();
        let result = op1.wrapping_sub(op2);
        self.set_flags_oszapc_sub_32(op1, op2, result);
        tracing::trace!("CMP EAX, imm32: {:#010x} - {:#010x}", op1, op2);
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

    /// TEST r16, r16
    pub fn test_ew_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr16(dst);
        let op2 = self.get_gpr16(src);
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!("TEST r16, r16: {:#06x} & {:#06x} = {:#06x}", op1, op2, result);
    }

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

    /// TEST AL, imm8
    pub fn test_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0); // AL
        let op2 = instr.ib();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_8(result);
        tracing::trace!("TEST AL, imm8: {:#04x} & {:#04x}", op1, op2);
    }

    /// TEST AX, imm16
    pub fn test_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0); // AX
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_16(result);
        tracing::trace!("TEST AX, imm16: {:#06x} & {:#06x}", op1, op2);
    }

    /// TEST EAX, imm32
    pub fn test_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0); // EAX
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_flags_oszapc_logic_32(result);
        tracing::trace!("TEST EAX, imm32: {:#010x} & {:#010x}", op1, op2);
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

    /// AND AL, imm8
    pub fn and_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0);
        let op2 = instr.ib();
        let result = op1 & op2;
        self.set_gpr8(0, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// AND AX, imm16
    pub fn and_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0);
        let op2 = instr.iw();
        let result = op1 & op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// AND EAX, imm32
    pub fn and_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0);
        let op2 = instr.id();
        let result = op1 & op2;
        self.set_gpr32(0, result);
        self.set_flags_oszapc_logic_32(result);
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
    // OR instructions
    // =========================================================================

    /// OR r8, r8
    pub fn or_gb_eb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let src = instr.src() as usize;
        let op1 = self.get_gpr8(dst);
        let op2 = self.get_gpr8(src);
        let result = op1 | op2;
        self.set_gpr8(dst, result);
        self.set_flags_oszapc_logic_8(result);
    }

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

    /// OR AL, imm8
    pub fn or_al_ib(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr8(0);
        let op2 = instr.ib();
        let result = op1 | op2;
        self.set_gpr8(0, result);
        self.set_flags_oszapc_logic_8(result);
    }

    /// OR AX, imm16
    pub fn or_ax_iw(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr16(0);
        let op2 = instr.iw();
        let result = op1 | op2;
        self.set_gpr16(0, result);
        self.set_flags_oszapc_logic_16(result);
    }

    /// OR EAX, imm32
    pub fn or_eax_id(&mut self, instr: &BxInstructionGenerated) {
        let op1 = self.get_gpr32(0);
        let op2 = instr.id();
        let result = op1 | op2;
        self.set_gpr32(0, result);
        self.set_flags_oszapc_logic_32(result);
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

    /// NOT r16
    pub fn not_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        self.set_gpr16(dst, !op1);
    }

    /// NOT r32
    pub fn not_ed_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr32(dst);
        self.set_gpr32(dst, !op1);
    }

    // =========================================================================
    // INC/DEC instructions
    // =========================================================================

    /// Update flags for INC (preserves CF)
    fn set_flags_oszap_inc_16(&mut self, result: u16, op1: u16) {
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = result == 0x8000; // Only overflow when 0x7FFF -> 0x8000
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        // CF is not affected by INC
        const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    /// Update flags for DEC (preserves CF)
    fn set_flags_oszap_dec_16(&mut self, result: u16, op1: u16) {
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = result == 0x7FFF && op1 == 0x8000;
        let af = ((op1 ^ 1 ^ result) & 0x10) != 0;
        let pf = (result as u8).count_ones() % 2 == 0;
        
        const MASK: u32 = (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;
        
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    /// INC r16
    pub fn inc_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.wrapping_add(1);
        self.set_gpr16(dst, result);
        self.set_flags_oszap_inc_16(result, op1);
        tracing::trace!("INC r16: {:#06x} + 1 = {:#06x}", op1, result);
    }

    /// DEC r16
    pub fn dec_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.dst() as usize;
        let op1 = self.get_gpr16(dst);
        let result = op1.wrapping_sub(1);
        self.set_gpr16(dst, result);
        self.set_flags_oszap_dec_16(result, op1);
        tracing::trace!("DEC r16: {:#06x} - 1 = {:#06x}", op1, result);
    }

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
}

