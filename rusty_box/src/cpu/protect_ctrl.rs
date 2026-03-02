//! Protected mode control instructions
//!
//! Based on Bochs protect_ctrl.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements LGDT, SGDT, LIDT, SIDT, SLDT, STR
//! (LLDT and LTR are in segment_ctrl_pro.rs)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// LGDT - Load Global Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:831-864
    pub fn lgdt_ms(&mut self, instr: &Instruction) -> Result<()> {
        // CPL must be 0 (Bochs protect_ctrl.cc:836-839)
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::debug!("LGDT: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr)?;
        let mut base = self.read_virtual_dword(seg, eaddr.wrapping_add(2))? as u64;

        // In 16-bit operand size mode, mask base to 24 bits (80286 compatibility)
        // Based on Bochs protect_ctrl.cc:858
        if instr.os32_l() == 0 {
            base &= 0x00FFFFFF;
        }

        self.gdtr.base = base;
        self.gdtr.limit = limit;
        tracing::trace!("LGDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    /// SGDT - Store Global Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:763-795
    pub fn sgdt_ms(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        self.write_virtual_word(seg, eaddr, self.gdtr.limit)?;
        self.write_virtual_dword(seg, eaddr.wrapping_add(2), self.gdtr.base as u32)?;
        Ok(())
    }

    /// LIDT - Load Interrupt Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:866-898
    pub fn lidt_ms(&mut self, instr: &Instruction) -> Result<()> {
        // CPL must be 0 (Bochs protect_ctrl.cc:871-874)
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::debug!("LIDT: CPL={} != 0, #GP(0)", cpl);
            return self.exception(super::cpu::Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr)?;
        let mut base = self.read_virtual_dword(seg, eaddr.wrapping_add(2))? as u64;

        // In 16-bit operand size mode, mask base to 24 bits
        // Based on Bochs protect_ctrl.cc:893
        if instr.os32_l() == 0 {
            base &= 0x00FFFFFF;
        }

        self.idtr.base = base;
        self.idtr.limit = limit;
        Ok(())
    }

    /// SIDT - Store Interrupt Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:797-829
    pub fn sidt_ms(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        self.write_virtual_word(seg, eaddr, self.idtr.limit)?;
        self.write_virtual_dword(seg, eaddr.wrapping_add(2), self.idtr.base as u32)?;
        Ok(())
    }

    /// SLDT - Store Local Descriptor Table Register
    /// Based on Bochs protect_ctrl.cc:286-328
    pub fn sldt_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // UMIP check (Bochs protect_ctrl.cc:293-297) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let val = self.ldtr.selector.value;
        if instr.mod_c0() {
            // Register destination
            self.set_gpr16(instr.dst() as usize, val);
        } else {
            // Memory destination — write 16-bit
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.write_virtual_word(seg, eaddr, val)?;
        }
        Ok(())
    }

    /// SMSW — Store Machine Status Word
    /// Based on Bochs crregs.cc:916-961
    pub fn smsw_ew(&mut self, instr: &Instruction) -> Result<()> {
        // UMIP check (Bochs crregs.cc:918-925) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }

        let msw = self.cr0.get32();

        if instr.mod_c0() {
            // Register form: writes 32-bit value (Bochs crregs.cc:928-935)
            if instr.os32_l() != 0 {
                self.set_gpr32(instr.dst() as usize, msw);
            } else {
                self.set_gpr16(instr.dst() as usize, msw as u16);
            }
        } else {
            // Memory form: always writes 16-bit (Bochs crregs.cc:937-958)
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.write_virtual_word(seg, eaddr, msw as u16)?;
        }
        Ok(())
    }

    /// STR - Store Task Register
    /// Based on Bochs protect_ctrl.cc:330-372
    pub fn str_ew(&mut self, instr: &Instruction) -> Result<()> {
        if !self.protected_mode() {
            return self.exception(super::cpu::Exception::Ud, 0);
        }
        // UMIP check (Bochs protect_ctrl.cc:337-341) — CR4.UMIP and CPL!=0 → #GP(0)
        if self.cr4.umip() {
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
            if cpl != 0 {
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        let val = self.tr.selector.value;
        if instr.mod_c0() {
            // Register destination
            self.set_gpr16(instr.dst() as usize, val);
        } else {
            // Memory destination — write 16-bit
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr32(instr);
            self.write_virtual_word(seg, eaddr, val)?;
        }
        Ok(())
    }
}
