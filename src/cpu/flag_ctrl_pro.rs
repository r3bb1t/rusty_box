use crate::cpu::{cpu::PendingEvent, BxCpuC, BxCpuIdTrait};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn handle_interrupt_mask_change(&mut self) {
        if self.get_if() != 0 {
            // EFLAGS.IF was set, unmask all affected events

            self.unmask_event(
                PendingEvent::VmxInterruptWindowExiting
                    | PendingEvent::PendingIntr
                    | PendingEvent::PendingLapicIntr
                    | PendingEvent::PendingVmxVirtualIntr,
            );

            if self.in_svm_guest {
                if true {
                    self.unmask_event(PendingEvent::SvmVirqPending);
                }
            }

            //  TODO: look into it
            //  if (!uintr_masked()) unmask_event(BX_EVENT_PENDING_UINTR);
        }
    }
}
