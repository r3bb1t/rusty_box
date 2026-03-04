use super::{cpu::CpuActivityState, cpuid::BxCpuIdTrait, eflags::EFlags, BxCpuC};

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

        // Priority 2: Trap on Task Switch (T flag in TSS)
        // Bochs event.cc:246-248
        if self.debug_trap & Self::BX_DEBUG_TRAP_TASK_SWITCH_BIT != 0 {
            self.debug_trap &= !Self::BX_DEBUG_TRAP_TASK_SWITCH_BIT;
            // Bochs: exception() calls longjmp, never returns.
            // We must propagate CpuLoopRestart by returning false.
            // The caller (cpu_loop_n) will restart the loop.
            if let Err(super::error::CpuError::CpuLoopRestart) =
                self.exception(super::cpu::Exception::Db, 0)
            {
                return false;
            }
        }

        // Priority 4: Debug trap exceptions (TF single-step, data/I/O breakpoints)
        // Bochs event.cc:312-324 — check inhibition FIRST, then debug_trap
        if !self.interrupts_inhibited(Self::BX_INHIBIT_DEBUG) {
            if self.debug_trap & 0xF000 != 0 {
                // BX_DEBUG_SINGLE_STEP_BIT or BX_DEBUG_DR_ACCESS_BIT set
                // Bochs: exception() longjmps — propagate restart
                if let Err(super::error::CpuError::CpuLoopRestart) =
                    self.exception(super::cpu::Exception::Db, 0)
                {
                    return false;
                }
            } else {
                self.debug_trap = 0;
            }
        }

        // Priority 5: External interrupts (Bochs event.cc:382-395)
        //
        // Matches Bochs HandleExtInterrupt(): when BX_EVENT_PENDING_INTR is set,
        // clear the event, check IF + inhibit, then call DEV_pic_iac() and
        // deliver via interrupt(). This is the critical path that allows the PIC
        // to deliver interrupts at instruction boundaries — without it, IRQs
        // could only be delivered at batch boundaries (causing starvation).
        if self.pending_event & Self::BX_EVENT_PENDING_INTR != 0 {
            self.pending_event &= !Self::BX_EVENT_PENDING_INTR;
            if self.eflags.contains(EFlags::IF_)
                && !self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS)
            {
                // Check LAPIC first (higher priority than PIC in APIC mode).
                // Bochs event.cc:383-388: BX_CPU_INTR is checked, then
                // service_local_apic() is called for APIC-mode delivery.
                #[cfg(feature = "bx_support_apic")]
                if self.lapic.intr {
                    let vector = self.lapic.acknowledge_int();
                    if vector > 0 {
                        self.diag_hae_intr_delivered += 1;
                        self.activity_state = CpuActivityState::Active;
                        self.ext = true;
                        let result = self.interrupt(vector, false, false, 0);
                        self.ext = false;
                        match result {
                            Ok(()) => {
                                self.prev_rip = self.rip() as u64;
                            }
                            Err(super::error::CpuError::CpuLoopRestart) => {
                                return false;
                            }
                            Err(_) => {}
                        }
                    }
                }
                // Then check PIC (legacy 8259 path, or when LAPIC has no pending interrupt)
                if !self.pic_ptr.is_null() {
                    let pic = unsafe { &mut *self.pic_ptr };
                    if pic.has_interrupt() {
                        let vector = pic.iac();
                        self.diag_hae_intr_delivered += 1;
                        // Wake from halt if needed
                        self.activity_state = CpuActivityState::Active;
                        // Mark as external interrupt (EXT=1)
                        self.ext = true;
                        // Deliver interrupt (matches Bochs interrupt() call in event.cc:389)
                        let result = self.interrupt(vector, false, false, 0);
                        self.ext = false;
                        match result {
                            Ok(()) => {
                                self.prev_rip = self.rip() as u64;
                            }
                            Err(super::error::CpuError::CpuLoopRestart) => {
                                return false; // Restart cpu_loop
                            }
                            Err(_) => {}
                        }
                    } else {
                        self.diag_hae_intr_pic_empty += 1;
                    }
                } else {
                    self.diag_hae_intr_no_pic += 1;
                }
            } else {
                self.diag_hae_intr_if_blocked += 1;
            }
        }

        // End of handleAsyncEvent: schedule TF->debug_trap for next boundary
        // Bochs event.cc:396-402
        if self.eflags.contains(EFlags::TF) {
            self.debug_trap |= Self::BX_DEBUG_SINGLE_STEP_BIT;
            self.async_event = 1;
        }

        // Bochs event.cc:428-433: Conditionally clear async_event
        // Only clear when no events remain pending (debug_trap, pending events, HRQ)
        let has_unmasked_events = (self.pending_event & !self.event_mask) != 0;
        if !has_unmasked_events && self.debug_trap == 0 {
            self.async_event = 0;
        }

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

        // For single processor, check if an external interrupt can wake us.
        // Matches Bochs event.cc:52-113
        //
        // In Bochs, handleWaitForEvent checks BX_EVENT_PENDING_INTR (if IF=1)
        // and NMI/SMI/INIT to decide whether to wake the CPU. For our
        // single-processor model, we check pending_event for PENDING_INTR
        // and also the LAPIC's intr flag.
        //
        // MWAIT_IF (ECX[0]=1 at MWAIT): wake on interrupt even when IF=0
        // (Bochs event.cc:73-80)
        let mwait_if = matches!(self.activity_state, CpuActivityState::MwaitIf);

        if self.pending_event & Self::BX_EVENT_PENDING_INTR != 0 {
            if self.eflags.contains(EFlags::IF_) || mwait_if {
                // External interrupt can wake from HLT/MWAIT
                self.activity_state = CpuActivityState::Active;
                self.inhibit_mask = 0;
                return false; // Continue to interrupt delivery
            }
        }
        // LAPIC interrupt can also wake from HLT/MWAIT
        #[cfg(feature = "bx_support_apic")]
        if self.lapic.intr && (self.eflags.contains(EFlags::IF_) || mwait_if) {
            self.activity_state = CpuActivityState::Active;
            self.inhibit_mask = 0;
            return false; // Continue to LAPIC interrupt delivery
        }

        // Monitor triggered by a write (wakeup_monitor set activity_state to Active)
        if matches!(self.activity_state, CpuActivityState::Active) {
            tracing::debug!("CPU activity_state became ACTIVE, waking up");
            self.inhibit_mask = 0;
            return false;
        }

        // Return from cpu_loop to allow other processing (matches single-CPU behavior)
        // In Bochs, BX_TICKN(10) advances time, then loops again
        // For our emulator, we return to allow GUI updates and device processing
        tracing::trace!("CPU halted, returning from cpu_loop to allow interrupt processing");
        // Bochs event.cc:68: clear inhibit_mask when waking from HLT
        self.inhibit_mask = 0;
        true
    }
}
