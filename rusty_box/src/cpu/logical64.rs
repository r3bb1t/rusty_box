//! 64-bit logical and comparison instructions for x86 CPU emulation
//!
//! Based on Bochs logical64.cc
//! Copyright (C) 2001-2019 The Bochs Project

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    eflags::EFlags,
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // =========================================================================
    // Flag update helpers
    // =========================================================================

    /// Update flags for 64-bit logical operations
    fn set_flags_oszapc_logic_64(&mut self, result: u64) {
        let sf = (result & 0x8000000000000000) != 0;
        let zf = result == 0;
        let pf = (result as u8).count_ones() % 2 == 0;

        self.eflags.remove(EFlags::LOGIC_MASK);

        if pf { self.eflags.insert(EFlags::PF); }
        if zf { self.eflags.insert(EFlags::ZF); }
        if sf { self.eflags.insert(EFlags::SF); }
    }

    // Note: 64-bit logical instructions are not yet implemented.
    // This file exists to match the original C++ structure.
    // When implementing 64-bit logical instructions, they should go here.
}
