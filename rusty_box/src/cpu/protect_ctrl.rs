//! Protected mode control instructions
//!
//! Based on Bochs protect_ctrl.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements LGDT, LIDT
//! (LLDT and LTR are in segment_ctrl_pro.rs)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    Result,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// LGDT - Load Global Descriptor Table Register
    pub fn lgdt_ms(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr)?;
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2))? as u64;
        self.gdtr.base = base;
        self.gdtr.limit = limit;
        tracing::trace!("LGDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    /// LIDT - Load Interrupt Descriptor Table Register
    pub fn lidt_ms(&mut self, instr: &Instruction) -> Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr)?;
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2))? as u64;
        self.idtr.base = base;
        self.idtr.limit = limit;
        Ok(())
    }
}
