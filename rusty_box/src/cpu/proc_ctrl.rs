use crate::cpu::{BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn handle_cpu_context_change(&mut self) {
        self.tlb_flush();

        self.invalidate_prefetch_q();
        self.invalidate_stack_cache();

        self.handle_interrupt_mask_change();

        self.handle_alignment_check();

        // FIXME: implement these
        // handleCpuModeChange();
        //
        // handleFpuMmxModeChange();
        // handleSseModeChange();
        // handleAvxModeChange();
    }

    fn handle_alignment_check(&mut self) {
        if self.cl() == 3 && self.cr0.am() && self.get_ac() != 0 {
            self.alignment_check_mask = 0xf;
        } else {
            self.alignment_check_mask = 0;
        }
    }

    /// Get the Time Stamp Counter value
    /// 
    /// # Arguments
    /// * `system_ticks` - Current system ticks from BxPcSystemC
    pub fn get_tsc(&self, system_ticks: u64) -> u64 {
        system_ticks.wrapping_add(self.tsc_adjust as u64)
    }

    /// Set the Time Stamp Counter to a specific value
    /// 
    /// # Arguments
    /// * `newval` - The value to set TSC to
    /// * `system_ticks` - Current system ticks from BxPcSystemC
    pub fn set_tsc(&mut self, newval: u64, system_ticks: u64) {
        // compute the correct setting of tsc_adjust so that a get_tsc()
        // will return newval
        self.tsc_adjust = newval.wrapping_sub(system_ticks) as i64
    }

    /// LIDT - Load Interrupt Descriptor Table Register
    /// Loads IDTR from a 6-byte memory operand (2-byte limit, 4-byte base)
    pub fn lidt_ms(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        use super::decoder::BxSegregs;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr);
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2)) as u64;
        self.idtr.base = base;
        self.idtr.limit = limit;
        tracing::trace!("LIDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }

    /// LGDT - Load Global Descriptor Table Register
    /// Loads GDTR from a 6-byte memory operand (2-byte limit, 4-byte base)
    pub fn lgdt_ms(&mut self, instr: &super::decoder::BxInstructionGenerated) -> crate::cpu::Result<()> {
        use super::decoder::BxSegregs;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr32(instr);
        let limit = self.read_virtual_word(seg, eaddr);
        let base = self.read_virtual_dword(seg, eaddr.wrapping_add(2)) as u64;
        self.gdtr.base = base;
        self.gdtr.limit = limit;
        tracing::trace!("LGDT: base={:#010x}, limit={:#06x}", base, limit);
        Ok(())
    }
}
