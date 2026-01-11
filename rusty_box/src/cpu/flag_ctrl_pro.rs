//! Flag Control Instructions (Protected Mode)
//!
//! This module contains flag manipulation helpers for protected mode.
//! 
//! Note: handle_interrupt_mask_change is defined in init.rs

use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Check if interrupts are enabled (EFLAGS.IF = 1)
    #[inline]
    pub fn interrupts_enabled(&self) -> bool {
        (self.eflags & (1 << 9)) != 0
    }
    
    /// Check if direction flag is set (EFLAGS.DF = 1)
    #[inline]
    pub fn direction_flag(&self) -> bool {
        (self.eflags & (1 << 10)) != 0
    }
}
