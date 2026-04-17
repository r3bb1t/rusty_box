use super::{
    cpu::{BxCpuC, CpuActivityState},
    cpuid::BxCpuIdTrait,
    Result,
};
use crate::config::BxPhyAddress;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Check if the given address range overlaps with the monitored address range
    pub fn is_monitor(&self, begin_addr: BxPhyAddress, len: u32) -> bool {
        if !self.monitor.armed() {
            return false;
        }

        const CACHE_LINE_SIZE: u64 = 64;
        let monitor_begin = self.monitor.monitor_addr;
        let monitor_end = monitor_begin + CACHE_LINE_SIZE - 1;

        let end_addr = begin_addr + len as u64;
        !(begin_addr >= monitor_end || end_addr <= monitor_begin)
    }

    /// Check if monitor should be triggered and wake up if so
    pub fn check_monitor(&mut self, begin_addr: BxPhyAddress, len: u32) -> Result<()> {
        if self.is_monitor(begin_addr, len) {
            self.wakeup_monitor();
        }
        Ok(())
    }

    /// Wake up from MWAIT state
    fn wakeup_monitor(&mut self) {
        // wakeup from MWAIT state
        if matches!(
            self.activity_state,
            CpuActivityState::Mwait | CpuActivityState::MwaitIf
        ) {
            self.activity_state = CpuActivityState::Active;
        }
        // clear monitor
        self.monitor.reset_monitor();
        // deactivate mwaitx timer if was active to avoid its redundant firing
        #[cfg(feature = "bx_support_apic")]
        {
            self.lapic.deactivate_mwaitx_timer();
        }
    }
}
