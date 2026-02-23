use super::{cpuid::BxCpuIdTrait, cpu::CpuActivityState, BxCpuC};

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    /// Handle async events - matches Bochs event.cc:205-436 handleAsyncEvent()
    /// Returns true if should return from cpu_loop
    pub(super) fn handle_async_event(&mut self) -> bool {
        // Check if CPU is in non-active state (HLT, MWAIT, etc.)
        // Matches Bochs event.cc:210-214
        if !matches!(self.activity_state, CpuActivityState::Active) {
            // For one processor, pass the time as quickly as possible until
            // an interrupt wakes up the CPU.
            if self.handle_wait_for_event() {
                return true; // Return to caller of cpu_loop
            }
        }

        // TODO: Priority 2-5 event handling (SMI, INIT, NMI, external interrupts, etc.)
        // For now, external interrupts are handled in the emulator's outer loop.

        // Matches Bochs event.cc:428-433:
        //   if (!(unmasked_events_pending || debug_trap || HRQ)) {
        //       async_event = 0;
        //   }
        // Clear async_event when no events remain pending.
        // Without this, BX_ASYNC_EVENT_STOP_TRACE stays set forever,
        // causing the inner trace loop to break after every instruction
        // (executed=1 per batch → usec=0 → tick_devices never called).
        self.async_event = 0;

        false // Continue execution
    }

    /// Handle wait for event - matches Bochs event.cc:handleWaitForEvent()
    /// Called when CPU is halted (HLT) or waiting (MWAIT)
    /// Returns true if should return from cpu_loop
    fn handle_wait_for_event(&mut self) -> bool {
        
        // For WAIT_FOR_SIPI, just return (matches Bochs event.cc:42-48)
        if matches!(self.activity_state, CpuActivityState::WaitForSipi) {
            tracing::debug!("CPU in WAIT_FOR_SIPI state, returning from cpu_loop");
            return true;
        }

        // For single processor, loop until interrupt wakes up CPU
        // Matches Bochs event.cc:52-113
        loop {
            // TODO: Check for pending interrupts/NMI/SMI
            // For now, just check if we should wake up
            // In real Bochs, it checks:
            // - BX_EVENT_PENDING_INTR | BX_EVENT_PENDING_LAPIC_INTR (if IF=1)
            // - BX_EVENT_NMI | BX_EVENT_SMI | BX_EVENT_INIT
            
            // For now, if activity_state became ACTIVE (e.g., from reset), wake up
            if matches!(self.activity_state, CpuActivityState::Active) {
                tracing::debug!("CPU activity_state became ACTIVE, waking up");
                break;
            }

            // Check if interrupts are enabled and pending
            // TODO: Check actual interrupt pending flags
            // For now, if IF=1, we'll assume we can wake up
            // (in real Bochs, we'd check BX_EVENT_PENDING_INTR)
            let if_flag = self.get_b_if();
            if if_flag != 0 {
                // In real Bochs, we'd check if interrupt is pending here
                // For now, just continue waiting
            }

            // TODO: Call BX_TICKN(10) to advance time
            // For now, we'll just return to let the emulator loop handle timing
            // This prevents infinite busy-wait loop
            
            // Return from cpu_loop to allow other processing (matches single-CPU behavior)
            // In Bochs, BX_TICKN(10) advances time, then loops again
            // For single CPU, this would loop forever until interrupt
            // For our emulator, we return to allow GUI updates and device processing
            tracing::debug!("CPU halted, returning from cpu_loop to allow interrupt processing");
            return true;
        }

        // Woke up from halt - clear activity state
        self.activity_state = CpuActivityState::Active;
        false // Continue execution
    }
}
