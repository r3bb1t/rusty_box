use crate::{
    cpu::{BxCpuC, BxCpuIdTrait},
    pc_system::bx_pc_system,
};

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

    fn get_tsc(&self) -> u64 {
        bx_pc_system().time_ticks() + self.tsc_adjust as u64
    }
    pub(super) fn set_tsc(&mut self, newval: u64) {
        // compute the correct setting of tsc_adjust so that a get_TSC()
        // will return newval
        self.tsc_adjust = (newval - bx_pc_system().time_ticks()) as i64
    }
}
