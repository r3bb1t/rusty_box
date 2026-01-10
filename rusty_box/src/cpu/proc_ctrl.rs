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
}
