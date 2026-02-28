//! Extended data transfer instructions for x86 CPU emulation
//!
//! Based on Bochs data_xfer16.cc, data_xfer32.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements LEA, XCHG, MOV segment, LES, LDS, CBW, CWD, CWDE, CDQ

use alloc::string::ToString;

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
    error::Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // LEA - Load Effective Address
    // =========================================================================

    /// LEA r16, m - Load effective address into 16-bit register
    pub fn lea_gw_m(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let eaddr = self.resolve_addr32(instr) as u16;
        self.set_gpr16(dst, eaddr);
        tracing::trace!("LEA16: reg{} = {:#06x}", dst, eaddr);
    }

    /// LEA r32, m - Load effective address into 32-bit register
    pub fn lea_gd_m(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let eaddr = self.resolve_addr32(instr);
        self.set_gpr32(dst, eaddr);
        tracing::trace!("LEA32: reg{} = {:#010x}", dst, eaddr);
    }

    // =========================================================================
    // XCHG - Exchange
    // =========================================================================

    /// XCHG r8, r/m8 - Exchange 8-bit values
    pub fn xchg_eb_gb(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val_dst = self.get_gpr8(dst);
        let val_src = self.get_gpr8(src);
        self.set_gpr8(dst, val_src);
        self.set_gpr8(src, val_dst);
        tracing::trace!(
            "XCHG8: reg{}={:#04x} <-> reg{}={:#04x}",
            dst,
            val_src,
            src,
            val_dst
        );
    }

    /// XCHG r16, r/m16 - Exchange 16-bit values
    pub fn xchg_ew_gw(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val_dst = self.get_gpr16(dst);
        let val_src = self.get_gpr16(src);
        self.set_gpr16(dst, val_src);
        self.set_gpr16(src, val_dst);
        tracing::trace!(
            "XCHG16: reg{}={:#06x} <-> reg{}={:#06x}",
            dst,
            val_src,
            src,
            val_dst
        );
    }

    /// XCHG r32, r/m32 - Exchange 32-bit values
    pub fn xchg_ed_gd(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val_dst = self.get_gpr32(dst);
        let val_src = self.get_gpr32(src);
        self.set_gpr32(dst, val_src);
        self.set_gpr32(src, val_dst);
        tracing::trace!(
            "XCHG32: reg{}={:#010x} <-> reg{}={:#010x}",
            dst,
            val_src,
            src,
            val_dst
        );
    }

    /// XCHG AX, r16 - Exchange AX with 16-bit register (short forms)
    pub fn xchg_ax_rw(&mut self, instr: &Instruction) {
        let reg = instr.meta_data[0] as usize;
        let ax = self.ax();
        let val = self.get_gpr16(reg);
        self.set_ax(val);
        self.set_gpr16(reg, ax);
    }

    /// XCHG EAX, r32 - Exchange EAX with 32-bit register (short forms)
    pub fn xchg_eax_rd(&mut self, instr: &Instruction) {
        let reg = instr.meta_data[0] as usize;
        let eax = self.eax();
        let val = self.get_gpr32(reg);
        self.set_eax(val);
        self.set_gpr32(reg, eax);
    }

    // =========================================================================
    // Unified XCHG dispatch (register vs memory)
    // =========================================================================

    /// XCHG r/m8, r8 - Unified dispatch based on mod_c0()
    pub fn xchg_eb_gb_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xchg_eb_gb(instr);
            Ok(())
        } else {
            self.xchg_eb_gb_m(instr)
        }
    }

    /// XCHG r/m16, r16 - Unified dispatch based on mod_c0()
    pub fn xchg_ew_gw_dispatch(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.xchg_ew_gw(instr);
            Ok(())
        } else {
            self.xchg_ew_gw_m(instr)
        }
    }

    // =========================================================================
    // MOV segment register operations
    // =========================================================================

    /// MOV r/m16, Sreg - Move segment register to r/m16
    pub fn mov_ew_sw(&mut self, instr: &Instruction) -> super::Result<()> {
        let src_seg = instr.meta_data[1] as usize;

        // Decoder should never give us invalid segment registers (6-7)
        // Valid segment registers: ES(0), CS(1), SS(2), DS(3), FS(4), GS(5)
        debug_assert!(
            src_seg <= 5,
            "Invalid segment register {} from decoder",
            src_seg
        );

        let seg_val = self.sregs[src_seg].selector.value;

        if instr.mod_c0() {
            // Register form: MOV r16, sreg
            let dst = instr.meta_data[0] as usize;
            self.set_gpr16(dst, seg_val);
            tracing::trace!("MOV: reg{} = seg{} ({:#06x})", dst, src_seg, seg_val);
        } else {
            // Memory form: MOV [mem], sreg
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.write_virtual_word(seg, eaddr, seg_val)?;
            tracing::trace!("MOV: [{:#x}] = seg{} ({:#06x})", eaddr, src_seg, seg_val);
        }
        Ok(())
    }

    /// MOV Sreg, r/m16 - Move r/m16 to segment register
    pub fn mov_sw_ew(&mut self, instr: &Instruction) -> Result<()> {
        let dst_seg = instr.meta_data[0] as usize;

        // Decoder should never give us invalid segment registers (6-7)
        // Valid segment registers: ES(0), CS(1), SS(2), DS(3), FS(4), GS(5)
        debug_assert!(
            dst_seg <= 5,
            "Invalid segment register {} from decoder",
            dst_seg
        );

        let new_sel = if instr.mod_c0() {
            // Register form: MOV sreg, r16
            let src = instr.meta_data[1] as usize;
            self.get_gpr16(src)
        } else {
            // Memory form: MOV sreg, [mem]
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_word(seg, eaddr)?
        };

        // Don't allow loading CS directly (would need special handling)
        if dst_seg == BxSegregs::Cs as usize {
            tracing::warn!("MOV to CS not allowed");
            return Err(super::error::CpuError::UnimplementedOpcode {
                opcode: "MOV CS, reg (use far jump/call instead)".to_string(),
            });
        }

        let seg = BxSegregs::from(dst_seg as u8);

        // Call load_seg_reg which handles both real and protected mode
        self.load_seg_reg(seg, new_sel)?;

        // MOV SS inhibits interrupts until next instruction boundary
        // (same as POP SS - Bochs data_xfer16.cc:124-129)
        if dst_seg == BxSegregs::Ss as usize {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS);
        }

        tracing::trace!("MOV: seg{} = {:#06x}", dst_seg, new_sel);
        Ok(())
    }

    // =========================================================================
    // Sign/Zero extension
    // =========================================================================

    /// CBW - Convert Byte to Word (AL -> AX)
    pub fn cbw(&mut self, _instr: &Instruction) {
        let al = self.al() as i8;
        self.set_ax(al as i16 as u16);
        tracing::trace!("CBW: AL={:#04x} -> AX={:#06x}", self.al(), self.ax());
    }

    /// CWD - Convert Word to Doubleword (AX -> DX:AX)
    pub fn cwd(&mut self, _instr: &Instruction) {
        let ax = self.ax() as i16;
        if ax < 0 {
            self.set_dx(0xFFFF);
        } else {
            self.set_dx(0);
        }
        tracing::trace!(
            "CWD: AX={:#06x} -> DX:AX={:#06x}:{:#06x}",
            ax,
            self.dx(),
            self.ax()
        );
    }

    /// CWDE - Convert Word to Doubleword Extended (AX -> EAX)
    pub fn cwde(&mut self, _instr: &Instruction) {
        let ax = self.ax() as i16;
        self.set_eax(ax as i32 as u32);
        tracing::trace!("CWDE: AX={:#06x} -> EAX={:#010x}", ax, self.eax());
    }

    /// CDQ - Convert Doubleword to Quadword (EAX -> EDX:EAX)
    pub fn cdq(&mut self, _instr: &Instruction) {
        let eax = self.eax() as i32;
        if eax < 0 {
            self.set_edx(0xFFFFFFFF);
        } else {
            self.set_edx(0);
        }
        tracing::trace!(
            "CDQ: EAX={:#010x} -> EDX:EAX={:#010x}:{:#010x}",
            eax,
            self.edx(),
            self.eax()
        );
    }

    // =========================================================================
    // XLAT - Table Lookup Translation
    // =========================================================================

    /// XLAT - Translate byte (AL = [BX+AL])
    pub fn xlat(&mut self, _instr: &Instruction) {
        let bx = self.ebx();
        let al = self.al() as u32;
        let eaddr = bx.wrapping_add(al);

        if let Ok(new_al) = self.read_virtual_byte(BxSegregs::Ds, eaddr) {
            self.set_al(new_al);
            tracing::trace!("XLAT: [BX+AL] = [{:#x}+{:#x}] = {:#04x}", bx, al, new_al);
        }
    }

    // =========================================================================
    // LAHF/SAHF - Load/Store AH from/to Flags
    // =========================================================================

    /// LAHF - Load AH from Flags (SF:ZF:0:AF:0:PF:1:CF)
    pub fn lahf(&mut self, _instr: &Instruction) {
        let flags = (self.eflags.bits() & 0xFF) as u8;
        // AH = SF:ZF:0:AF:0:PF:1:CF (bits 7,6,4,2,0 from flags, bit 1 always 1)
        let ah = (flags & 0xD5) | 0x02;
        self.set_ah(ah);
        tracing::trace!("LAHF: AH = {:#04x}", ah);
    }

    /// SAHF - Store AH into Flags
    pub fn sahf(&mut self, _instr: &Instruction) {
        let ah = self.ah();
        // Only modify SF, ZF, AF, PF, CF (bits 7,6,4,2,0)
        self.eflags = EFlags::from_bits_retain((self.eflags.bits() & !0xD5) | ((ah as u32) & 0xD5));
        tracing::trace!("SAHF: flags = {:#010x}", self.eflags.bits());
    }

    // =========================================================================
    // MOV with immediate values (16-bit versions)
    // =========================================================================

    /// MOV r16, imm16
    pub fn mov_rw_iw(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let imm = instr.iw();

        self.set_gpr16(dst, imm);
        tracing::trace!("MOV: reg{} = {:#06x}", dst, imm);
    }

    /// MOV r8, imm8
    pub fn mov_rb_ib(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let imm = instr.ib();
        self.set_gpr8(dst, imm);
        tracing::trace!("MOV: reg{} = {:#04x}", dst, imm);
    }

    /// MOV r16, r/m16 (register to register)
    pub fn mov_gw_ew_r(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr16(src);
        self.set_gpr16(dst, val);
        tracing::trace!("MOV16: reg{} = reg{} ({:#06x})", dst, src, val);
    }

    /// MOV r/m16, r16 (register to register)
    /// Opcode 0x89 (16-bit): decoder swaps: meta_data[0] = rm (DEST), meta_data[1] = nnn (SOURCE)
    pub fn mov_ew_gw_r(&mut self, instr: &Instruction) {
        let val = self.get_gpr16(instr.meta_data[1] as usize); // nnn = source
        self.set_gpr16(instr.meta_data[0] as usize, val); // rm = destination
        tracing::trace!(
            "MOV16: reg{} = reg{} ({:#06x})",
            instr.meta_data[0],
            instr.meta_data[1],
            val
        );
    }

    /// MOV r8, r/m8 (register to register)
    pub fn mov_gb_eb_r(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src);
        self.set_gpr8(dst, val);
        tracing::trace!("MOV8: reg{} = reg{} ({:#04x})", dst, src, val);
    }

    /// MOV r/m8, r8 (register to register)
    /// Opcode 0x88: 8-bit does NOT match decoder swap — meta_data[0]=nnn=source, meta_data[1]=rm=dest
    pub fn mov_eb_gb_r(&mut self, instr: &Instruction) {
        let val = self.get_gpr8(instr.meta_data[0] as usize); // nnn = source
        self.set_gpr8(instr.meta_data[1] as usize, val); // rm = destination
        tracing::trace!(
            "MOV8: reg{} = reg{} ({:#04x})",
            instr.meta_data[1],
            instr.meta_data[0],
            val
        );
    }

    // =========================================================================
    // 8-bit MOV memory forms (matching C++ data_xfer8.cc)
    // =========================================================================

    /// MOV r/m8, imm8 (memory form)
    /// Matching C++ data_xfer8.cc:75-82 MOV_EbIbM
    pub fn mov_eb_ib_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.write_virtual_byte(seg, eaddr, instr.ib())?;
        tracing::trace!("MOV8 mem: [{:?}:{:#x}] = {:#04x}", seg, eaddr, instr.ib());
        Ok(())
    }

    /// MOV r/m8, r8 (memory form)
    /// Matching C++ data_xfer8.cc:34-41 MOV_EbGbM
    /// 8-bit: no decoder swap, meta_data[0]=nnn=source register, dst()=meta_data[0]
    pub fn mov_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let src_reg = instr.dst() as usize; // dst()=[0]=nnn=source register for 8-bit store
        let extend8bit_l = instr.extend8bit_l();
        let val8 = self.read_8bit_regx(src_reg, extend8bit_l);

        self.write_virtual_byte(seg, eaddr, val8)?;
        tracing::trace!(
            "MOV8 mem: [{:?}:{:#x}] = reg{} ({:#04x})",
            seg,
            eaddr,
            src_reg,
            val8
        );
        Ok(())
    }

    /// MOV r8, r/m8 (memory form)
    /// Matching C++ data_xfer8.cc:43-51 MOV_GbEbM
    pub fn mov_gb_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let val8 = self.read_virtual_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let extend8bit_l = instr.extend8bit_l();

        self.write_8bit_regx(dst_reg, extend8bit_l, val8);
        tracing::trace!(
            "MOV8 mem: reg{} = [{:?}:{:#x}] ({:#04x})",
            dst_reg,
            seg,
            eaddr,
            val8
        );
        Ok(())
    }

    /// XCHG r/m8, r8 (memory form)
    /// Matching C++ data_xfer8.cc:99-110 XCHG_EbGbM
    /// Note: always locked (read_RMW_virtual_byte)
    pub fn xchg_eb_gb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_rmw_virtual_byte(seg, eaddr)?; // always locked
        let src_reg = instr.dst() as usize; // dst()=[0]=nnn=register (8-bit XCHG not in decoder swap list)
        let extend8bit_l = instr.extend8bit_l();
        let op2 = self.read_8bit_regx(src_reg, extend8bit_l);

        self.write_rmw_linear_byte(op2);
        self.write_8bit_regx(src_reg, extend8bit_l, op1);
        tracing::trace!(
            "XCHG8 mem: [{:?}:{:#x}]={:#04x} <-> reg{}={:#04x}",
            seg,
            eaddr,
            op2,
            src_reg,
            op1
        );
        Ok(())
    }

    // =========================================================================
    // Helper functions for 8-bit memory operations
    // =========================================================================

    // write_virtual_byte is defined in access.rs

    // write_8bit_regx is defined in logical8.rs to avoid duplicate definitions

    // Helper methods (resolve_addr32, read_8bit_regx, etc.) are defined in logical8.rs
    // to avoid duplicate definitions across multiple impl blocks

    // =========================================================================
    // MOVZX/MOVSX - Move with Zero/Sign Extension
    // =========================================================================

    /// MOVZX r16, r/m8 — unified dispatch (register or memory form)
    pub fn movzx_gw_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.movzx_gw_eb_r(instr);
            Ok(())
        } else {
            self.movzx_gw_eb_m(instr)
        }
    }

    /// MOVZX r32, r/m8 - Move with zero-extend
    pub fn movzx_gd_eb(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src) as u32;
        self.set_gpr32(dst, val);
    }

    /// MOVZX r32, r/m16 - Move with zero-extend
    pub fn movzx_gd_ew(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr16(src) as u32;
        self.set_gpr32(dst, val);
    }

    /// MOVSX r16, r/m8 — unified dispatch (register or memory form)
    pub fn movsx_gw_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.movsx_gw_eb_r(instr);
            Ok(())
        } else {
            self.movsx_gw_eb_m(instr)
        }
    }

    /// MOVSX r32, r/m8 - Move with sign-extend (legacy meta_data form, superseded by _r/_m variants)
    pub fn movsx_gd_eb_legacy(&mut self, instr: &Instruction) {
        let dst = instr.meta_data[0] as usize;
        let src = instr.meta_data[1] as usize;
        let val = self.get_gpr8(src) as i8 as i32 as u32;
        self.set_gpr32(dst, val);
    }

    // =========================================================================
    // 16-bit MOV memory forms (matching C++ data_xfer16.cc)
    // =========================================================================

    /// MOV r/m16, imm16 (memory form)
    /// Matching C++ data_xfer16.cc:27-33 MOV_EwIwM
    pub fn mov_ew_iw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.write_virtual_word(seg, eaddr, instr.iw())?;
        tracing::trace!("MOV16 mem: [{:?}:{:#x}] = {:#06x}", seg, eaddr, instr.iw());
        Ok(())
    }

    /// MOV r16, imm16 (register form)
    /// Matching C++ data_xfer16.cc:35-40 MOV_EwIwR
    pub fn mov_ew_iw_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;

        self.set_gpr16(dst, instr.iw());
        tracing::trace!("MOV16: reg{} = {:#06x}", dst, instr.iw());
    }

    /// MOV r/m16, r16 (memory form)
    /// Matching C++ data_xfer16.cc:42-49 MOV_EwGwM
    /// Decoder swaps for 16/32-bit store: src() = meta_data[1] = nnn = SOURCE register
    pub fn mov_ew_gw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let src_reg = instr.src() as usize;
        let val16 = self.get_gpr16(src_reg);

        self.write_virtual_word(seg, eaddr, val16)?;
        tracing::trace!(
            "MOV16 mem: [{:?}:{:#x}] = reg{} ({:#06x})",
            seg,
            eaddr,
            src_reg,
            val16
        );
        Ok(())
    }

    /// MOV r16, r/m16 (memory form)
    /// Matching C++ data_xfer16.cc:58-65 MOV_GwEwM
    /// MOV r16, r/m16 (memory form)
    /// Matching C++ data_xfer16.cc:58-65 MOV_GwEwM
    pub fn mov_gw_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let val16 = self.read_virtual_word(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        self.set_gpr16(dst_reg, val16);
        tracing::trace!(
            "MOV16 mem: reg{} = [{:?}:{:#x}] ({:#06x})",
            dst_reg,
            seg,
            eaddr,
            val16
        );
        Ok(())
    }

    /// XCHG r/m16, r16 (memory form)
    /// Matching C++ data_xfer16.cc:202-210 XCHG_EwGwM
    /// Note: always locked (read_RMW_virtual_word)
    /// reg field (dst()) = register operand for XCHG memory form
    pub fn xchg_ew_gw_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_rmw_virtual_word(seg, eaddr)?; // always locked
        let src_reg = instr.dst() as usize;
        let op2 = self.get_gpr16(src_reg);

        self.write_rmw_linear_word(op2);
        self.set_gpr16(src_reg, op1);
        tracing::trace!(
            "XCHG16 mem: [{:?}:{:#x}]={:#06x} <-> reg{}={:#06x}",
            seg,
            eaddr,
            op2,
            src_reg,
            op1
        );
        Ok(())
    }

    /// MOVZX r16, r/m8 (memory form)
    /// Matching C++ data_xfer16.cc:158-168 MOVZX_GwEbM
    /// Zero extend byte op2 into word op1
    pub fn movzx_gw_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_8 = self.read_virtual_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr16(dst_reg, op2_8 as u16);
        tracing::trace!(
            "MOVZX16 mem: reg{} = [{:?}:{:#x}] ({:#04x})",
            dst_reg,
            seg,
            eaddr,
            op2_8
        );
        Ok(())
    }

    /// MOVZX r16, r8 (register form)
    /// Matching C++ data_xfer16.cc:170-178 MOVZX_GwEbR
    /// Zero extend byte op2 into word op1
    pub fn movzx_gw_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;

        self.set_gpr16(dst_reg, op2_8 as u16);
        tracing::trace!("MOVZX16: reg{} = reg{} ({:#04x})", dst_reg, src_reg, op2_8);
    }

    /// MOVSX r16, r/m8 (memory form)
    /// Matching C++ data_xfer16.cc:180-190 MOVSX_GwEbM
    /// Sign extend byte op2 into word op1
    pub fn movsx_gw_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_8 = self.read_virtual_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let val16 = (op2_8 as i8 as i16) as u16; // sign extend byte to word

        self.set_gpr16(dst_reg, val16);
        tracing::trace!(
            "MOVSX16 mem: reg{} = [{:?}:{:#x}] ({:#04x} -> {:#06x})",
            dst_reg,
            seg,
            eaddr,
            op2_8,
            val16
        );
        Ok(())
    }

    /// MOVSX r16, r8 (register form)
    /// Matching C++ data_xfer16.cc:192-200 MOVSX_GwEbR
    /// Sign extend byte op2 into word op1
    pub fn movsx_gw_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;
        let val16 = (op2_8 as i8 as i16) as u16; // sign extend byte to word

        self.set_gpr16(dst_reg, val16);
        tracing::trace!(
            "MOVSX16: reg{} = reg{} ({:#04x} -> {:#06x})",
            dst_reg,
            src_reg,
            op2_8,
            val16
        );
    }

    // =========================================================================
    // CMOV - Conditional Move (16-bit)
    // =========================================================================
    // Note: CMOV accesses a memory source operand (read), regardless
    //       of whether condition is true or not.  Thus, exceptions may
    //       occur even if the MOV does not take place.
    // Matching C++ data_xfer16.cc:241-371

    /// Conditional move if overflow (OF=1)
    /// Matching C++ data_xfer16.cc:245-251 CMOVO_GwEwR
    pub fn cmovo_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_of() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNO_GwEwR - Conditional move if not overflow (OF=0)
    pub fn cmovno_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_of() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVB_GwEwR - Conditional move if below/carry (CF=1)
    pub fn cmovb_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_cf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNB_GwEwR - Conditional move if not below/no carry (CF=0)
    pub fn cmovnb_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_cf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVZ_GwEwR - Conditional move if zero/equal (ZF=1)
    pub fn cmovz_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_zf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNZ_GwEwR - Conditional move if not zero/not equal (ZF=0)
    pub fn cmovnz_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVBE_GwEwR - Conditional move if below or equal (CF=1 or ZF=1)
    pub fn cmovbe_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_cf() || self.get_zf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNBE_GwEwR - Conditional move if not below or equal/above (CF=0 and ZF=0)
    pub fn cmovnbe_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_cf() && !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVS_GwEwR - Conditional move if sign (SF=1)
    pub fn cmovs_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_sf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNS_GwEwR - Conditional move if not sign (SF=0)
    pub fn cmovns_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_sf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVP_GwEwR - Conditional move if parity/parity even (PF=1)
    pub fn cmovp_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_pf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNP_GwEwR - Conditional move if no parity/parity odd (PF=0)
    pub fn cmovnp_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_pf() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVL_GwEwR - Conditional move if less (SF != OF)
    pub fn cmovl_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_sf() != self.get_of() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNL_GwEwR - Conditional move if not less/greater or equal (SF == OF)
    pub fn cmovnl_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_sf() == self.get_of() {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVLE_GwEwR - Conditional move if less or equal (ZF=1 or SF!=OF)
    pub fn cmovle_gw_ew_r(&mut self, instr: &Instruction) {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    /// CMOVNLE_GwEwR - Conditional move if not less or equal/greater (ZF=0 and SF==OF)
    pub fn cmovnle_gw_ew_r(&mut self, instr: &Instruction) {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let src_reg = instr.src() as usize;
            let val16 = self.get_gpr16(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr16(dst_reg, val16);
        }
    }

    // =========================================================================
    // Helper functions for 16-bit memory operations
    // =========================================================================

    // write_virtual_word is defined in access.rs

    // read_virtual_word is defined in access.rs

    // read_rmw_virtual_word is defined in logical16.rs to avoid duplicate definitions

    // write_rmw_linear_word is defined in logical16.rs to avoid duplicate definitions

    // =========================================================================
    // 32-bit MOV memory forms (matching C++ data_xfer32.cc)
    // =========================================================================

    /// MOV r/m32, imm32 (memory form)
    /// Matching C++ data_xfer32.cc:27-33 MOV_EdIdM
    pub fn mov_ed_id_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());

        self.write_virtual_dword(seg, eaddr, instr.id())?;
        tracing::trace!("MOV32 mem: [{:?}:{:#x}] = {:#010x}", seg, eaddr, instr.id());
        Ok(())
    }

    /// MOV r32, imm32 (register form)
    /// Matching C++ data_xfer32.cc:35-40 MOV_EdIdR
    /// Note: BX_CLEAR_64BIT_HIGH is handled in set_gpr32
    pub fn mov_ed_id_r(&mut self, instr: &Instruction) {
        let dst = instr.dst() as usize;

        self.set_gpr32(dst, instr.id());
        tracing::trace!("MOV32: reg{} = {:#010x}", dst, instr.id());
    }

    /// MOV r/m32, r32 (memory form)
    ///
    /// Writes a 32-bit value from the source register to memory.
    /// The memory address is computed from the ModRM byte and segment register.
    ///
    /// Matching C++ data_xfer32.cc:42-49 BX_CPU_C::MOV32_EdGdM
    pub fn mov32_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let src_reg = instr.src() as usize;
        let val32 = self.get_gpr32(src_reg);

        self.write_virtual_dword(seg, eaddr, val32)?;
        tracing::trace!(
            "MOV32 mem: [{:?}:{:#x}] = reg{} ({:#010x})",
            seg,
            eaddr,
            src_reg,
            val32
        );
        Ok(())
    }

    /// MOV r32, r/m32 (memory form)
    /// Matching C++ data_xfer32.cc:67-75 MOV32_GdEdM
    /// Note: BX_CLEAR_64BIT_HIGH is handled in set_gpr32
    pub fn mov32_gd_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let val32 = self.read_virtual_dword(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOV32 mem: reg{} = [{:?}:{:#x}] ({:#010x})",
            dst_reg,
            seg,
            eaddr,
            val32
        );
        Ok(())
    }

    /// MOV r32, r/m32 (memory form with SS segment override)
    /// Matching C++ data_xfer32.cc:77-85 MOV32S_GdEdM
    /// Uses stack_read_dword instead of read_virtual_dword
    pub fn mov32s_gd_ed_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let val32 = self.stack_read_dword(eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOV32S mem: reg{} = [SS:{:#x}] ({:#010x})",
            dst_reg,
            eaddr,
            val32
        );
        Ok(())
    }

    /// MOV r/m32, r32 (memory form with SS segment override)
    ///
    /// This handler is used when MOV instruction has an SS segment override prefix.
    /// It uses stack_write_dword instead of write_virtual_dword to write memory through
    /// the SS segment, which is important for stack operations.
    ///
    /// Matching C++ data_xfer32.cc:51-58 BX_CPU_C::MOV32S_EdGdM
    ///
    /// # Operation
    /// Writes a 32-bit value from the source register to SS:offset.
    pub fn mov32s_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let src_reg = instr.src() as usize;
        let val32 = self.get_gpr32(src_reg);

        self.stack_write_dword(eaddr, val32)?;
        tracing::trace!(
            "MOV32S mem: [SS:{:#x}] = reg{} ({:#010x})",
            eaddr,
            src_reg,
            val32
        );
        Ok(())
    }

    /// XCHG r/m32, r32 (memory form)
    /// Matching C++ data_xfer32.cc:198-207 XCHG_EdGdM
    /// Note: always locked (read_RMW_virtual_dword)
    /// XCHG 0x87 is NOT in decoder swap list, so [0]=nnn=register, [1]=rm
    pub fn xchg_ed_gd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op1 = self.read_rmw_virtual_dword(seg, eaddr)?; // always locked
        let src_reg = instr.dst() as usize; // dst()=[0]=nnn=register operand
        let op2 = self.get_gpr32(src_reg);

        self.write_rmw_linear_dword(op2);
        self.set_gpr32(src_reg, op1);
        tracing::trace!(
            "XCHG32 mem: [{:?}:{:#x}]={:#010x} <-> reg{}={:#010x}",
            seg,
            eaddr,
            op2,
            src_reg,
            op1
        );
        Ok(())
    }

    /// MOVZX r32, r/m8 (memory form)
    /// Matching C++ data_xfer32.cc:110-120 MOVZX_GdEbM
    /// Zero extend byte op2 into dword op1
    pub fn movzx_gd_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_8 = self.read_virtual_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, op2_8 as u32);
        tracing::trace!(
            "MOVZX32 mem: reg{} = [{:?}:{:#x}] ({:#04x})",
            dst_reg,
            seg,
            eaddr,
            op2_8
        );
        Ok(())
    }

    /// MOVZX r32, r8 (register form)
    /// Matching C++ data_xfer32.cc:122-130 MOVZX_GdEbR
    /// Zero extend byte op2 into dword op1
    pub fn movzx_gd_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, op2_8 as u32);
        tracing::trace!("MOVZX32: reg{} = reg{} ({:#04x})", dst_reg, src_reg, op2_8);
    }

    /// MOVZX r32, r/m16 (memory form)
    /// Matching C++ data_xfer32.cc:132-142 MOVZX_GdEwM
    /// Zero extend word op2 into dword op1
    pub fn movzx_gd_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, op2_16 as u32);
        tracing::trace!(
            "MOVZX32 mem: reg{} = [{:?}:{:#x}] ({:#06x})",
            dst_reg,
            seg,
            eaddr,
            op2_16
        );
        Ok(())
    }

    /// MOVZX r32, r16 (register form)
    /// Matching C++ data_xfer32.cc:144-152 MOVZX_GdEwR
    /// Zero extend word op2 into dword op1
    pub fn movzx_gd_ew_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_16 = self.get_gpr16(src_reg);
        let dst_reg = instr.dst() as usize;

        self.set_gpr32(dst_reg, op2_16 as u32);
        tracing::trace!("MOVZX32: reg{} = reg{} ({:#06x})", dst_reg, src_reg, op2_16);
    }

    /// MOVSX r32, r/m8 (memory form)
    /// Matching C++ data_xfer32.cc:154-164 MOVSX_GdEbM
    /// Sign extend byte op2 into dword op1
    pub fn movsx_gd_eb_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_8 = self.read_virtual_byte(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let val32 = (op2_8 as i8 as i32) as u32; // sign extend byte to dword

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOVSX32 mem: reg{} = [{:?}:{:#x}] ({:#04x} -> {:#010x})",
            dst_reg,
            seg,
            eaddr,
            op2_8,
            val32
        );
        Ok(())
    }

    /// MOVSX r32, r8 (register form)
    /// Matching C++ data_xfer32.cc:166-174 MOVSX_GdEbR
    /// Sign extend byte op2 into dword op1
    pub fn movsx_gd_eb_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let extend8bit_l = instr.extend8bit_l();
        let op2_8 = self.read_8bit_regx(src_reg, extend8bit_l);
        let dst_reg = instr.dst() as usize;
        let val32 = (op2_8 as i8 as i32) as u32; // sign extend byte to dword

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOVSX32: reg{} = reg{} ({:#04x} -> {:#010x})",
            dst_reg,
            src_reg,
            op2_8,
            val32
        );
    }

    /// MOVSX r32, r/m16 (memory form)
    /// Matching C++ data_xfer32.cc:176-186 MOVSX_GdEwM
    /// Sign extend word op2 into dword op1
    pub fn movsx_gd_ew_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let op2_16 = self.read_virtual_word(seg, eaddr)?;
        let dst_reg = instr.dst() as usize;
        let val32 = (op2_16 as i16 as i32) as u32; // sign extend word to dword

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOVSX32 mem: reg{} = [{:?}:{:#x}] ({:#06x} -> {:#010x})",
            dst_reg,
            seg,
            eaddr,
            op2_16,
            val32
        );
        Ok(())
    }

    /// MOVSX r32, r16 (register form)
    /// Matching C++ data_xfer32.cc:188-196 MOVSX_GdEwR
    /// Sign extend word op2 into dword op1
    pub fn movsx_gd_ew_r(&mut self, instr: &Instruction) {
        let src_reg = instr.src() as usize;
        let op2_16 = self.get_gpr16(src_reg);
        let dst_reg = instr.dst() as usize;
        let val32 = (op2_16 as i16 as i32) as u32; // sign extend word to dword

        self.set_gpr32(dst_reg, val32);
        tracing::trace!(
            "MOVSX32: reg{} = reg{} ({:#06x} -> {:#010x})",
            dst_reg,
            src_reg,
            op2_16,
            val32
        );
    }

    // =========================================================================
    // CMOV - Conditional Move (32-bit)
    // =========================================================================
    // Note: CMOV accesses a memory source operand (read), regardless
    //       of whether condition is true or not.  Thus, exceptions may
    //       occur even if the MOV does not take place.
    // Matching C++ data_xfer32.cc:219-381

    /// Conditional move if overflow (OF=1)
    /// Matching C++ data_xfer32.cc:223-231 CMOVO_GdEdR
    /// Always clear upper part of the register (BX_CLEAR_64BIT_HIGH)
    pub fn cmovo_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_of() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        // Always clear high 64 bits (matching C++ line 228)
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not overflow (OF=0)
    /// Matching C++ data_xfer32.cc:233-241 CMOVNO_GdEdR
    /// Always clear upper part of the register
    pub fn cmovno_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_of() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if below/carry (CF=1)
    /// Matching C++ data_xfer32.cc:243-251 CMOVB_GdEdR
    /// Always clear upper part of the register
    pub fn cmovb_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_cf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not below/no carry (CF=0)
    /// Matching C++ data_xfer32.cc:253-261 CMOVNB_GdEdR
    /// Always clear upper part of the register
    pub fn cmovnb_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_cf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if zero/equal (ZF=1)
    /// Matching C++ data_xfer32.cc:263-271 CMOVZ_GdEdR
    /// Always clear upper part of the register
    pub fn cmovz_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_zf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not zero/not equal (ZF=0)
    /// Matching C++ data_xfer32.cc:273-281 CMOVNZ_GdEdR
    /// Always clear upper part of the register
    pub fn cmovnz_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if below or equal (CF=1 or ZF=1)
    /// Matching C++ data_xfer32.cc:283-291 CMOVBE_GdEdR
    /// Always clear upper part of the register
    pub fn cmovbe_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_cf() || self.get_zf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not below or equal/above (CF=0 and ZF=0)
    /// Matching C++ data_xfer32.cc:293-301 CMOVNBE_GdEdR
    /// Always clear upper part of the register
    pub fn cmovnbe_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_cf() && !self.get_zf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if sign (SF=1)
    /// Matching C++ data_xfer32.cc:303-311 CMOVS_GdEdR
    /// Always clear upper part of the register
    pub fn cmovs_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_sf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not sign (SF=0)
    /// Matching C++ data_xfer32.cc:313-321 CMOVNS_GdEdR
    /// Always clear upper part of the register
    pub fn cmovns_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_sf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if parity/parity even (PF=1)
    /// Matching C++ data_xfer32.cc:323-331 CMOVP_GdEdR
    /// Always clear upper part of the register
    pub fn cmovp_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_pf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if no parity/parity odd (PF=0)
    /// Matching C++ data_xfer32.cc:333-341 CMOVNP_GdEdR
    /// Always clear upper part of the register
    pub fn cmovnp_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_pf() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if less (SF != OF)
    /// Matching C++ data_xfer32.cc:343-351 CMOVL_GdEdR
    /// Always clear upper part of the register
    pub fn cmovl_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_sf() != self.get_of() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not less/greater or equal (SF == OF)
    /// Matching C++ data_xfer32.cc:353-361 CMOVNL_GdEdR
    /// Always clear upper part of the register
    pub fn cmovnl_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_sf() == self.get_of() {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if less or equal (ZF=1 or SF!=OF)
    /// Matching C++ data_xfer32.cc:363-371 CMOVLE_GdEdR
    /// Always clear upper part of the register
    pub fn cmovle_gd_ed_r(&mut self, instr: &Instruction) {
        if self.get_zf() || (self.get_sf() != self.get_of()) {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    /// Conditional move if not less or equal/greater (ZF=0 and SF==OF)
    /// Matching C++ data_xfer32.cc:373-381 CMOVNLE_GdEdR
    /// Always clear upper part of the register
    pub fn cmovnle_gd_ed_r(&mut self, instr: &Instruction) {
        if !self.get_zf() && (self.get_sf() == self.get_of()) {
            let src_reg = instr.src() as usize;
            let val32 = self.get_gpr32(src_reg);
            let dst_reg = instr.dst() as usize;
            self.set_gpr32(dst_reg, val32);
        }
        self.bx_clear_64bit_high(instr.dst() as usize);
    }

    // =========================================================================
    // Helper functions for 32-bit memory operations
    // =========================================================================

    // write_virtual_dword is defined in access.rs

    // read_virtual_dword is defined in access.rs

    // Helper methods (read_rmw_virtual_dword, write_rmw_linear_dword) are defined in logical32.rs to avoid duplicate definitions

    // =========================================================================
    // Unified mod_c0 dispatch wrappers
    // =========================================================================
    // These wrappers dispatch to the _r (register) or _m (memory) form
    // based on the mod_c0 flag in the decoded instruction.

    /// MOV r8, r/m8 - unified dispatch
    pub fn mov_gb_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.mov_gb_eb_r(instr);
            Ok(())
        } else {
            self.mov_gb_eb_m(instr)
        }
    }

    /// MOV r/m8, r8 - unified dispatch
    pub fn mov_eb_gb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.mov_eb_gb_r(instr);
            Ok(())
        } else {
            self.mov_eb_gb_m(instr)
        }
    }

    /// MOV r/m8, imm8 - unified dispatch
    /// Note: R form is mov_rb_ib (different naming convention)
    pub fn mov_eb_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.mov_rb_ib(instr);
            Ok(())
        } else {
            self.mov_eb_ib_m(instr)
        }
    }

    /// MOV r16, r/m16 - unified dispatch
    pub fn mov_gw_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.mov_gw_ew_r(instr);
            Ok(())
        } else {
            self.mov_gw_ew_m(instr)
        }
    }

    /// MOV r/m16, r16 - unified dispatch
    pub fn mov_ew_gw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.mov_ew_gw_r(instr);
            Ok(())
        } else {
            self.mov_ew_gw_m(instr)
        }
    }

    /// MOV r/m16, imm16 - unified dispatch
    /// Note: R form is mov_rw_iw (different naming convention)
    pub fn mov_ew_iw(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.mov_rw_iw(instr);
            Ok(())
        } else {
            self.mov_ew_iw_m(instr)
        }
    }

    /// MOVSX r32, r/m8 - unified dispatch
    pub fn movsx_gd_eb(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.movsx_gd_eb_r(instr);
            Ok(())
        } else {
            self.movsx_gd_eb_m(instr)
        }
    }

    /// MOVSX r32, r/m16 - unified dispatch
    pub fn movsx_gd_ew(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            self.movsx_gd_ew_r(instr);
            Ok(())
        } else {
            self.movsx_gd_ew_m(instr)
        }
    }

    // =========================================================================
    // LES/LDS - Load Far Pointer
    // =========================================================================
    // Based on Bochs segment_ctrl.cc load_segw/load_segd helpers

    /// LES r16, m16:16 - Load ES:r16 from memory far pointer
    /// Matching Bochs segment_ctrl.cc LES_GwMp -> load_segw(i, BX_SEG_REG_ES)
    pub fn les_gw_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_16 = self.read_virtual_word(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;

        self.load_seg_reg(BxSegregs::Es, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr16(dst, reg_16);
        tracing::trace!("LES16: reg{} = {:#06x}, ES = {:#06x}", dst, reg_16, segsel);
        Ok(())
    }

    /// LES r32, m16:32 - Load ES:r32 from memory far pointer
    /// Matching Bochs segment_ctrl.cc LES_GdMp -> load_segd(i, BX_SEG_REG_ES)
    pub fn les_gd_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_32 = self.read_virtual_dword(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(4))?;
        self.load_seg_reg(BxSegregs::Es, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, reg_32);
        tracing::trace!("LES32: reg{} = {:#010x}, ES = {:#06x}", dst, reg_32, segsel);
        Ok(())
    }

    /// LDS r16, m16:16 - Load DS:r16 from memory far pointer
    /// Matching Bochs segment_ctrl.cc LDS_GwMp -> load_segw(i, BX_SEG_REG_DS)
    pub fn lds_gw_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_16 = self.read_virtual_word(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;

        self.load_seg_reg(BxSegregs::Ds, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr16(dst, reg_16);
        tracing::trace!("LDS16: reg{} = {:#06x}, DS = {:#06x}", dst, reg_16, segsel);
        Ok(())
    }

    /// LDS r32, m16:32 - Load DS:r32 from memory far pointer
    /// Matching Bochs segment_ctrl.cc LDS_GdMp -> load_segd(i, BX_SEG_REG_DS)
    pub fn lds_gd_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_32 = self.read_virtual_dword(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(4))?;
        self.load_seg_reg(BxSegregs::Ds, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, reg_32);
        tracing::trace!("LDS32: reg{} = {:#010x}, DS = {:#06x}", dst, reg_32, segsel);
        Ok(())
    }

    /// LSS r16, m16:16 - Load SS:r16 from memory far pointer
    pub fn lss_gw_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_16 = self.read_virtual_word(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;
        self.load_seg_reg(BxSegregs::Ss, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr16(dst, reg_16);
        tracing::trace!("LSS16: reg{} = {:#06x}, SS = {:#06x}", dst, reg_16, segsel);
        Ok(())
    }

    /// LSS r32, m16:32 - Load SS:r32 from memory far pointer
    pub fn lss_gd_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_32 = self.read_virtual_dword(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(4))?;
        self.load_seg_reg(BxSegregs::Ss, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, reg_32);
        tracing::trace!("LSS32: reg{} = {:#010x}, SS = {:#06x}", dst, reg_32, segsel);
        Ok(())
    }

    /// LFS r16, m16:16 - Load FS:r16 from memory far pointer
    pub fn lfs_gw_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_16 = self.read_virtual_word(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;
        self.load_seg_reg(BxSegregs::Fs, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr16(dst, reg_16);
        tracing::trace!("LFS16: reg{} = {:#06x}, FS = {:#06x}", dst, reg_16, segsel);
        Ok(())
    }

    /// LFS r32, m16:32 - Load FS:r32 from memory far pointer
    pub fn lfs_gd_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_32 = self.read_virtual_dword(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(4))?;
        self.load_seg_reg(BxSegregs::Fs, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, reg_32);
        tracing::trace!("LFS32: reg{} = {:#010x}, FS = {:#06x}", dst, reg_32, segsel);
        Ok(())
    }

    /// LGS r16, m16:16 - Load GS:r16 from memory far pointer
    pub fn lgs_gw_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_16 = self.read_virtual_word(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(2))?;
        self.load_seg_reg(BxSegregs::Gs, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr16(dst, reg_16);
        tracing::trace!("LGS16: reg{} = {:#06x}, GS = {:#06x}", dst, reg_16, segsel);
        Ok(())
    }

    /// LGS r32, m16:32 - Load GS:r32 from memory far pointer
    pub fn lgs_gd_mp(&mut self, instr: &Instruction) -> Result<()> {
        let eaddr = self.resolve_addr32(instr);
        let seg = BxSegregs::from(instr.seg());
        let reg_32 = self.read_virtual_dword(seg, eaddr)?;
        let segsel = self.read_virtual_word(seg, eaddr.wrapping_add(4))?;
        self.load_seg_reg(BxSegregs::Gs, segsel)?;
        let dst = instr.dst() as usize;
        self.set_gpr32(dst, reg_32);
        tracing::trace!("LGS32: reg{} = {:#010x}, GS = {:#06x}", dst, reg_32, segsel);
        Ok(())
    }
}
