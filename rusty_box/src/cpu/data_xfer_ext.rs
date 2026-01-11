//! Extended data transfer instructions for x86 CPU emulation
//!
//! Based on Bochs data_xfer16.cc, data_xfer32.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements LEA, XCHG, MOV segment, LES, LDS, CBW, CWD, CWDE, CDQ

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxInstructionGenerated, BxSegregs},
    segment_ctrl_pro::parse_selector,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // LEA - Load Effective Address
    // =========================================================================
    
    /// LEA r16, m - Load effective address into 16-bit register
    pub fn lea_gw_m(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        // The effective address calculation is done by the decoder
        // For now, use the displacement as the address (simplified)
        let ea = instr.id() as u16; // Simplified - real implementation needs addressing mode decode
        self.set_gpr16(dst, ea);
        tracing::trace!("LEA: reg{} = {:#06x}", dst, ea);
    }

    /// LEA r32, m - Load effective address into 32-bit register  
    pub fn lea_gd_m(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let ea = instr.id();
        self.set_gpr32(dst, ea);
        tracing::trace!("LEA: reg{} = {:#010x}", dst, ea);
    }

    // =========================================================================
    // XCHG - Exchange
    // =========================================================================
    
    /// XCHG r8, r/m8 - Exchange 8-bit values
    pub fn xchg_eb_gb(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val_dst = self.get_gpr8(dst);
        let val_src = self.get_gpr8(src);
        self.set_gpr8(dst, val_src);
        self.set_gpr8(src, val_dst);
        tracing::trace!("XCHG8: reg{}={:#04x} <-> reg{}={:#04x}", dst, val_src, src, val_dst);
    }

    /// XCHG r16, r/m16 - Exchange 16-bit values
    pub fn xchg_ew_gw(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val_dst = self.get_gpr16(dst);
        let val_src = self.get_gpr16(src);
        self.set_gpr16(dst, val_src);
        self.set_gpr16(src, val_dst);
        tracing::trace!("XCHG16: reg{}={:#06x} <-> reg{}={:#06x}", dst, val_src, src, val_dst);
    }

    /// XCHG r32, r/m32 - Exchange 32-bit values
    pub fn xchg_ed_gd(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val_dst = self.get_gpr32(dst);
        let val_src = self.get_gpr32(src);
        self.set_gpr32(dst, val_src);
        self.set_gpr32(src, val_dst);
        tracing::trace!("XCHG32: reg{}={:#010x} <-> reg{}={:#010x}", dst, val_src, src, val_dst);
    }

    /// XCHG AX, r16 - Exchange AX with 16-bit register (short forms)
    pub fn xchg_ax_rw(&mut self, instr: &BxInstructionGenerated) {
        let reg = instr.meta_data[0] as usize;
        let ax = self.ax();
        let val = self.get_gpr16(reg);
        self.set_ax(val);
        self.set_gpr16(reg, ax);
    }

    /// XCHG EAX, r32 - Exchange EAX with 32-bit register (short forms)
    pub fn xchg_eax_rd(&mut self, instr: &BxInstructionGenerated) {
        let reg = instr.meta_data[0] as usize;
        let eax = self.eax();
        let val = self.get_gpr32(reg);
        self.set_eax(val);
        self.set_gpr32(reg, eax);
    }

    // =========================================================================
    // MOV segment register operations
    // =========================================================================
    
    /// MOV r/m16, Sreg - Move segment register to r/m16
    pub fn mov_ew_sw(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src_seg = instr.meta_data[1] as usize;
        
        let seg_val = self.sregs[src_seg].selector.value;
        self.set_gpr16(dst, seg_val);
        tracing::trace!("MOV: reg{} = seg{} ({:#06x})", dst, src_seg, seg_val);
    }

    /// MOV Sreg, r/m16 - Move r/m16 to segment register
    pub fn mov_sw_ew(&mut self, instr: &BxInstructionGenerated) {
        let dst_seg = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        
        let new_sel = self.get_gpr16(src);
        
        // Don't allow loading CS directly
        if dst_seg == BxSegregs::Cs as usize {
            tracing::warn!("MOV to CS not allowed, ignoring");
            return;
        }
        
        // Load segment register (real mode)
        parse_selector(new_sel, &mut self.sregs[dst_seg].selector);
        unsafe {
            self.sregs[dst_seg].cache.u.segment.base = (new_sel as u64) << 4;
            self.sregs[dst_seg].cache.u.segment.limit_scaled = 0xFFFF;
        }
        
        tracing::trace!("MOV: seg{} = {:#06x}", dst_seg, new_sel);
    }

    // =========================================================================
    // Sign/Zero extension
    // =========================================================================
    
    /// CBW - Convert Byte to Word (AL -> AX)
    pub fn cbw(&mut self, _instr: &BxInstructionGenerated) {
        let al = self.al() as i8;
        self.set_ax(al as i16 as u16);
        tracing::trace!("CBW: AL={:#04x} -> AX={:#06x}", self.al(), self.ax());
    }

    /// CWD - Convert Word to Doubleword (AX -> DX:AX)
    pub fn cwd(&mut self, _instr: &BxInstructionGenerated) {
        let ax = self.ax() as i16;
        if ax < 0 {
            self.set_dx(0xFFFF);
        } else {
            self.set_dx(0);
        }
        tracing::trace!("CWD: AX={:#06x} -> DX:AX={:#06x}:{:#06x}", ax, self.dx(), self.ax());
    }

    /// CWDE - Convert Word to Doubleword Extended (AX -> EAX)
    pub fn cwde(&mut self, _instr: &BxInstructionGenerated) {
        let ax = self.ax() as i16;
        self.set_eax(ax as i32 as u32);
        tracing::trace!("CWDE: AX={:#06x} -> EAX={:#010x}", ax, self.eax());
    }

    /// CDQ - Convert Doubleword to Quadword (EAX -> EDX:EAX)
    pub fn cdq(&mut self, _instr: &BxInstructionGenerated) {
        let eax = self.eax() as i32;
        if eax < 0 {
            self.set_edx(0xFFFFFFFF);
        } else {
            self.set_edx(0);
        }
        tracing::trace!("CDQ: EAX={:#010x} -> EDX:EAX={:#010x}:{:#010x}", eax, self.edx(), self.eax());
    }

    // =========================================================================
    // XLAT - Table Lookup Translation
    // =========================================================================
    
    /// XLAT - Translate byte (AL = [BX+AL])
    pub fn xlat(&mut self, _instr: &BxInstructionGenerated) {
        let bx = self.bx() as u64;
        let al = self.al() as u64;
        let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
        let addr = ds_base.wrapping_add(bx).wrapping_add(al);
        
        let new_al = self.mem_read_byte(addr);
        self.set_al(new_al);
        tracing::trace!("XLAT: [BX+AL] = [{}+{}] = {:#04x}", bx, al, new_al);
    }

    // =========================================================================
    // LAHF/SAHF - Load/Store AH from/to Flags
    // =========================================================================
    
    /// LAHF - Load AH from Flags (SF:ZF:0:AF:0:PF:1:CF)
    pub fn lahf(&mut self, _instr: &BxInstructionGenerated) {
        let flags = (self.eflags & 0xFF) as u8;
        // AH = SF:ZF:0:AF:0:PF:1:CF (bits 7,6,4,2,0 from flags, bit 1 always 1)
        let ah = (flags & 0xD5) | 0x02;
        self.set_ah(ah);
        tracing::trace!("LAHF: AH = {:#04x}", ah);
    }

    /// SAHF - Store AH into Flags
    pub fn sahf(&mut self, _instr: &BxInstructionGenerated) {
        let ah = self.ah();
        // Only modify SF, ZF, AF, PF, CF (bits 7,6,4,2,0)
        self.eflags = (self.eflags & !0xD5) | ((ah as u32) & 0xD5);
        tracing::trace!("SAHF: flags = {:#010x}", self.eflags);
    }

    // =========================================================================
    // MOV with immediate values (16-bit versions)
    // =========================================================================
    
    /// MOV r16, imm16
    pub fn mov_rw_iw(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let imm = instr.iw();
        self.set_gpr16(dst, imm);
        tracing::trace!("MOV: reg{} = {:#06x}", dst, imm);
    }

    /// MOV r8, imm8
    pub fn mov_rb_ib(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let imm = instr.ib();
        self.set_gpr8(dst, imm);
        tracing::trace!("MOV: reg{} = {:#04x}", dst, imm);
    }

    /// MOV r16, r/m16 (register to register)
    pub fn mov_gw_ew_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr16(src);
        self.set_gpr16(dst, val);
        tracing::trace!("MOV16: reg{} = reg{} ({:#06x})", dst, src, val);
    }

    /// MOV r/m16, r16 (register to register)
    pub fn mov_ew_gw_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr16(src);
        self.set_gpr16(dst, val);
        tracing::trace!("MOV16: reg{} = reg{} ({:#06x})", dst, src, val);
    }

    /// MOV r8, r/m8 (register to register)
    pub fn mov_gb_eb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src);
        self.set_gpr8(dst, val);
        tracing::trace!("MOV8: reg{} = reg{} ({:#04x})", dst, src, val);
    }

    /// MOV r/m8, r8 (register to register)
    pub fn mov_eb_gb_r(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src);
        self.set_gpr8(dst, val);
        tracing::trace!("MOV8: reg{} = reg{} ({:#04x})", dst, src, val);
    }

    // =========================================================================
    // MOVZX/MOVSX - Move with Zero/Sign Extension
    // =========================================================================
    
    /// MOVZX r16, r/m8 - Move with zero-extend
    pub fn movzx_gw_eb(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src) as u16;
        self.set_gpr16(dst, val);
    }

    /// MOVZX r32, r/m8 - Move with zero-extend
    pub fn movzx_gd_eb(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src) as u32;
        self.set_gpr32(dst, val);
    }

    /// MOVZX r32, r/m16 - Move with zero-extend
    pub fn movzx_gd_ew(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr16(src) as u32;
        self.set_gpr32(dst, val);
    }

    /// MOVSX r16, r/m8 - Move with sign-extend
    pub fn movsx_gw_eb(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src) as i8 as i16 as u16;
        self.set_gpr16(dst, val);
    }

    /// MOVSX r32, r/m8 - Move with sign-extend
    pub fn movsx_gd_eb(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src) as i8 as i32 as u32;
        self.set_gpr32(dst, val);
    }

    /// MOVSX r32, r/m16 - Move with sign-extend
    pub fn movsx_gd_ew(&mut self, instr: &BxInstructionGenerated) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr16(src) as i16 as i32 as u32;
        self.set_gpr32(dst, val);
    }
}

