//! Flag control instructions
//! Matching Bochs flag_ctrl.cc -- CLC, STC, CMC, CLD, STD, CLI, STI, SALC

use super::decoder::BxSegregs;
use super::eflags::EFlags;
use crate::cpu::{BxCpuC, BxCpuIdTrait};

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

    /// CLI - Clear Interrupt Flag
    /// Based on Bochs flag_ctrl.cc
    pub(super) fn cli(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let iopl = self.eflags.iopl() as u32;

        if self.protected_mode() {
            // PVI: Protected Virtual Interrupts (CR4.PVI && CPL==3)
            if self.cr4.pvi() {
                let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl as u32;
                if cpl == 3
                    && iopl < 3 {
                        // Clear VIF instead of IF
                        self.eflags.remove(EFlags::VIF);
                        return Ok(());
                    }
            }
            // Check IOPL >= CPL
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl as u32;
            if iopl < cpl {
                tracing::debug!("CLI: IOPL < CPL in protected mode");
                self.exception(super::cpu::Exception::Gp, 0)?;
            }
        } else if self.v8086_mode()
            && iopl != 3 {
                if self.cr4.vme() {
                    // Clear VIF instead of IF
                    self.eflags.remove(EFlags::VIF);
                    return Ok(());
                }
                tracing::debug!("CLI: IOPL != 3 in v8086 mode");
                self.exception(super::cpu::Exception::Gp, 0)?;
            }

        self.eflags.remove(EFlags::IF_);
        // Bochs flag_ctrl.cc: handleInterruptMaskChange() after clearing IF
        self.handle_interrupt_mask_change();
        Ok(())
    }

    /// STI - Set Interrupt Flag
    /// Based on Bochs flag_ctrl.cc
    pub(super) fn sti(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        let iopl = self.eflags.iopl() as u32;

        if self.protected_mode() {
            // PVI: Protected Virtual Interrupts (CR4.PVI)
            if self.cr4.pvi() {
                let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl as u32;
                if cpl == 3 && iopl < 3 {
                    if !self.eflags.contains(EFlags::VIP) {
                        // Set VIF
                        self.eflags.insert(EFlags::VIF);
                        return Ok(());
                    }
                    tracing::debug!("STI: #GP(0) in VME mode");
                    self.exception(super::cpu::Exception::Gp, 0)?;
                }
            }
            // Check CPL <= IOPL
            let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl as u32;
            if cpl > iopl {
                tracing::debug!("STI: CPL > IOPL in protected mode");
                self.exception(super::cpu::Exception::Gp, 0)?;
            }
        } else if self.v8086_mode()
            && iopl != 3 {
                if self.cr4.vme() && !self.eflags.contains(EFlags::VIP) {
                    // Set VIF
                    self.eflags.insert(EFlags::VIF);
                    return Ok(());
                }
                tracing::debug!("STI: IOPL != 3 in v8086 mode");
                self.exception(super::cpu::Exception::Gp, 0)?;
            }

        // Only inhibit if IF was previously clear
        if !self.eflags.contains(EFlags::IF_) {
            self.eflags.insert(EFlags::IF_);
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS);
            // Bochs flag_ctrl.cc: handleInterruptMaskChange() after setting IF
            self.handle_interrupt_mask_change();
        }

        Ok(())
    }

    pub(super) fn cld(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.remove(EFlags::DF);
        Ok(())
    }

    pub(super) fn std_(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        self.eflags.insert(EFlags::DF);
        Ok(())
    }

    /// SALC - Set AL from Carry (undocumented, opcode 0xD6)
    /// Based on Bochs flag_ctrl.cc
    pub(super) fn salc(&mut self, _instr: &super::decoder::Instruction) -> crate::cpu::Result<()> {
        if self.eflags.contains(EFlags::CF) {
            self.set_al(0xFF);
        } else {
            self.set_al(0x00);
        }
        Ok(())
    }
}
