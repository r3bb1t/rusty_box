//! Flag control instructions
//! Matching Bochs flag_ctrl.cc -- CLC, STC, CMC, CLD, STD, CLI, STI

use crate::cpu::{BxCpuC, BxCpuIdTrait};
use super::eflags::EFlags;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn clc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.remove(EFlags::CF);
        Ok(())
    }

    pub(super) fn stc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.insert(EFlags::CF);
        Ok(())
    }

    pub(super) fn cmc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.toggle(EFlags::CF);
        Ok(())
    }

    pub(super) fn cli(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.remove(EFlags::IF_);
        tracing::debug!("CLI: Interrupts disabled");
        Ok(())
    }

    pub(super) fn sti(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.insert(EFlags::IF_);
        tracing::debug!("STI: Interrupts enabled");
        Ok(())
    }

    pub(super) fn cld(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.remove(EFlags::DF);
        tracing::debug!("CLD: Direction flag cleared");
        Ok(())
    }

    pub(super) fn std_(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.insert(EFlags::DF);
        tracing::debug!("STD: Direction flag set");
        Ok(())
    }
}
