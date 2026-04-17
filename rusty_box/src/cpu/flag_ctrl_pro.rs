//! Flag Control Instructions (Protected Mode)
//!
//! This module contains flag manipulation helpers for protected mode.
//! Based on Bochs flag_ctrl_pro.cc
//!
//! Note: handle_interrupt_mask_change is defined in init.rs

use super::eflags::EFlags;
use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Check if interrupts are enabled (EFLAGS.IF = 1)
    #[inline]
    pub fn interrupts_enabled(&self) -> bool {
        self.eflags.contains(EFlags::IF_)
    }

    /// Check if direction flag is set (EFLAGS.DF = 1)
    #[inline]
    pub fn direction_flag(&self) -> bool {
        self.eflags.contains(EFlags::DF)
    }

    /// Set EFLAGS with full side effects.
    /// Based on Bochs flag_ctrl_pro.cc setEFlags()
    pub(super) fn set_eflags_internal(&mut self, mut new_eflags: u32) {
        // Bochs: if (long_mode()) new_eflags &= ~EFlagsVMMask;
        if self.long_mode() {
            new_eflags &= !(EFlags::VM.bits());
        }
        let old = self.eflags;
        self.eflags = EFlags::from_bits_retain(new_eflags);
        let new_flags = self.eflags;

        // RF set => invalidate prefetch queue
        if new_flags.contains(EFlags::RF) {
            self.invalidate_prefetch_q();
        }

        // TF set => schedule debug trap
        if new_flags.contains(EFlags::TF) {
            self.async_event = 1;
        }

        // IF changed => handle interrupt mask change
        if old.contains(EFlags::IF_) != new_flags.contains(EFlags::IF_) {
            self.handle_interrupt_mask_change();
        }

        // AC or VM changed => recheck alignment
        self.handle_alignment_check();

        // VM changed => handle CPU mode change
        if old.contains(EFlags::VM) != new_flags.contains(EFlags::VM) {
            self.handle_cpu_mode_change();
        }
    }

    /// Write EFLAGS with change mask and side effects.
    /// Based on Bochs flag_ctrl_pro.cc writeEFlags(flags, changeMask)
    ///
    /// Only bits in `change_mask` (AND'd with support mask) are modified.
    /// Triggers side effects: TF/IF/AC/VM/RF checks.
    pub(super) fn write_eflags(&mut self, flags: u32, change_mask: u32) {
        let change_mask = change_mask & EFlags::SUPPORT_MASK.bits();
        let new_eflags = (self.eflags.bits() & !change_mask) | (flags & change_mask);
        self.set_eflags_internal(new_eflags);
    }

    /// write_flags — 16-bit version used by real-mode IRET16 and protected-mode IRET16.
    /// Based on Bochs flag_ctrl_pro.cc write_flags(flags16, change_IOPL, change_IF)
    pub(super) fn write_flags(&mut self, flags16: u16, change_iopl: bool, change_if: bool) {
        // Base changeMask: CF|PF|AF|ZF|SF|TF|DF|OF|NT
        let mut change = EFlags::CF
            .union(EFlags::PF)
            .union(EFlags::AF)
            .union(EFlags::ZF)
            .union(EFlags::SF)
            .union(EFlags::TF)
            .union(EFlags::DF)
            .union(EFlags::OF)
            .union(EFlags::NT);
        if change_iopl {
            change = change.union(EFlags::IOPL_MASK);
        }
        if change_if {
            change = change.union(EFlags::IF_);
        }
        self.write_eflags(flags16 as u32, change.bits());
    }
}
