use super::{cpu::CpuActivityState, cpuid::BxCpuIdTrait, eflags::EFlags, BxCpuC};

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'c, I, T> {
    /// Handle async events - matches Bochs event.cc handleAsyncEvent()
    /// Returns true if should return from cpu_loop
    pub(super) fn handle_async_event(
        &mut self,
        pic: Option<&mut crate::pic::BxPicC>,
        mut dma: Option<&mut crate::dma::BxDmaC>,
    ) -> bool {
        // Check if CPU is in non-active state (HLT, MWAIT, etc.)
        // Matches Bochs event.cc
        if !matches!(self.activity_state, CpuActivityState::Active) {
            // For one processor, pass the time as quickly as possible until
            // an interrupt wakes up the CPU.
            if self.handle_wait_for_event(dma.as_deref_mut()) {
                return true; // Return to caller of cpu_loop
            }
        }

        // Priority 2: Trap on Task Switch (T flag in TSS)
        // Bochs event.cc — deliver #DB BEFORE clearing the bit
        // so that DR6 still has BT set when the handler reads it
        if self.debug_trap & Self::BX_DEBUG_TRAP_TASK_SWITCH_BIT != 0 {
            // Bochs: exception() calls longjmp, never returns.
            // We must propagate CpuLoopRestart by returning false.
            // The caller (cpu_loop_n) will restart the loop.
            if let Err(super::error::CpuError::CpuLoopRestart) =
                self.exception(super::cpu::Exception::Db, 0)
            {
                self.debug_trap &= !Self::BX_DEBUG_TRAP_TASK_SWITCH_BIT;
                return false;
            }
            self.debug_trap &= !Self::BX_DEBUG_TRAP_TASK_SWITCH_BIT;
        }

        // Priority 3: External Hardware Interventions (Bochs event.cc)
        //   FLUSH, STOPCLK, SMI, INIT

        // SMI (Bochs event.cc): enter System Management Mode.
        // Not implemented — single-CPU DLX/Alpine don't trigger SMI.
        // Bochs: clear_event(BX_EVENT_SMI); enter_system_management_mode();
        if self.is_unmasked_event_pending(Self::BX_EVENT_SMI) {
            self.clear_event(Self::BX_EVENT_SMI);
            tracing::trace!("SMI event cleared (SMM not implemented)");
        }

        // INIT (Bochs event.cc): reset CPU via reset(BX_RESET_SOFTWARE).
        // Used by multiprocessor startup (INIT-SIPI-SIPI sequence).
        // Not implemented — single-CPU emulation only.
        // Bochs: clear_event(BX_EVENT_INIT); reset(BX_RESET_SOFTWARE);
        if self.is_unmasked_event_pending(Self::BX_EVENT_INIT) {
            self.clear_event(Self::BX_EVENT_INIT);
            tracing::trace!("INIT event cleared (SMP not implemented)");
        }

        // Priority 4: Debug trap exceptions (TF single-step, data/I/O breakpoints)
        // Bochs event.cc — check inhibition FIRST, then debug_trap
        if !self.interrupts_inhibited(Self::BX_INHIBIT_DEBUG) {
            // Bochs event.cc: OR code breakpoint matches into debug_trap
            self.debug_trap |= self.code_breakpoint_match(self.prev_rip);
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

        // Priority 5: External interrupts (Bochs event.cc)
        //
        // Bochs structure:
        //   1. if interrupts_inhibited(BX_INHIBIT_INTERRUPTS) → skip all
        //   2. else if is_unmasked_event_pending(NMI) → deliver NMI
        //   3. else if is_unmasked_event_pending(PENDING_INTR|LAPIC_INTR) → HandleExtInterrupt()
        //
        // HandleExtInterrupt delivers exactly ONE interrupt (LAPIC or PIC),
        // not both. LAPIC has higher priority than PIC.
        //
        // The event_mask mechanism (managed by handleInterruptMaskChange) gates
        // PENDING_INTR and LAPIC_INTR based on IF: when IF=0, they are masked in
        // event_mask, so is_unmasked_event_pending returns false. The event
        // stays in pending_event and is delivered when IF becomes 1 again.
        //
        // Critical: do NOT clear PENDING_INTR here — it is cleared only by
        // pic.iac() → BX_CLEAR_INTR → clear_event(). If cleared here and
        // IF=0, the interrupt would be permanently lost.

        // Bochs event.cc Priority-5 window-exits run when external interrupts
        // are not inhibited:
        //   - NMI-window exit fires before NMI delivery when the window is
        //     open (NMI not currently blocked).
        //   - Interrupt-window exit fires before external-interrupt delivery
        //     when there is no pending NMI to deliver first.
        // Both are pin-/proc-based VMX exits and stay cold outside VMX guest.
        if self.in_vmx_guest && !self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS) {
            if (self.event_mask & Self::BX_EVENT_NMI) == 0 {
                match self.vmexit_check_nmi_window() {
                    Ok(true) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => {
                        tracing::warn!("VMX NMI-window vmexit failed: {:?}", e);
                    }
                    Ok(false) => {}
                }
            }
            if !self.is_unmasked_event_pending(Self::BX_EVENT_NMI) {
                match self.vmexit_check_interrupt_window() {
                    Ok(true) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => {
                        tracing::warn!("VMX interrupt-window vmexit failed: {:?}", e);
                    }
                    Ok(false) => {}
                }
            }
        }

        if self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS) {
            // STI/MOV SS shadow — skip all external interrupts this boundary
            // (Bochs event.cc)
        } else if self.is_unmasked_event_pending(Self::BX_EVENT_NMI) {
            // NMI delivery (Bochs event.cc)
            self.clear_event(Self::BX_EVENT_NMI);
            self.ext = true;
            // Bochs vmexit.cc VMexit_Event(BX_NMI, 2, 0, 0): pin-based NMI
            // exit fires before delivery into the guest IDT.
            if self.in_vmx_guest {
                match self.vmexit_check_nmi() {
                    Ok(true) => {
                        self.ext = false;
                        self.mask_event(Self::BX_EVENT_NMI);
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Ok(false) => {}
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.ext = false;
                        self.mask_event(Self::BX_EVENT_NMI);
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => {
                        tracing::warn!("VMX NMI vmexit failed: {:?}", e);
                    }
                }
            }
            self.mask_event(Self::BX_EVENT_NMI); // Block further NMIs until IRET
            self.activity_state = CpuActivityState::Active;
            let result = self.interrupt(2, super::exception::InterruptType::Nmi, false, false, 0); // NMI vector = 2
            self.ext = false;
            match result {
                Ok(()) => {
                    self.prev_rip = self.rip();
                }
                Err(super::error::CpuError::CpuLoopRestart) => {
                    self.prev_rip = self.rip();
                    return false;
                }
                Err(e) => {
                    tracing::warn!("NMI delivery failed: {:?}", e);
                }
            }
        } else if self.is_unmasked_event_pending(
            Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR,
        ) {
            // HandleExtInterrupt (Bochs event.cc).
            //
            // Bochs vmexit.cc VMexit_ExtInterrupt: with EXTERNAL_INTERRUPT_VMEXIT
            // set and INTA_ON_VMEXIT clear, the VMEXIT happens BEFORE the
            // controller is acknowledged so the interrupt remains pending in
            // the host PIC/LAPIC for re-delivery. The INTA_ON_VMEXIT path
            // acknowledges first and routes through vmexit_check_event_intr
            // below so the vector lands in exit_intr_info.
            if self.in_vmx_guest {
                match self.vmexit_check_ext_intr_no_ack() {
                    Ok(true) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Ok(false) => {}
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => {
                        tracing::warn!("VMX ext-intr no-ack vmexit failed: {:?}", e);
                    }
                }
            }

            // Deliver exactly ONE interrupt: LAPIC first, then PIC.
            let mut delivered = false;

            // Check LAPIC first (higher priority than PIC in APIC mode)
            if !delivered && self.lapic.intr {
                // Clear event before acknowledge — acknowledge_int() calls
                // service_local_apic() which may re-signal if more IRQs pending.
                self.clear_event(Self::BX_EVENT_PENDING_LAPIC_INTR);
                let vector = self.lapic.acknowledge_int();
                if vector > 0 {
                    #[cfg(debug_assertions)] {
                        self.diag_hae_intr_delivered += 1;
                        self.diag_iac_vectors[vector as usize] += 1;
                    }
                    self.activity_state = CpuActivityState::Active;
                    self.ext = true;
                    // Bochs vmexit.cc VMexit_Event(BX_EXTERNAL_INTERRUPT, vector,
                    // 0, 0): post-ack pin-based exit when INTA_ON_VMEXIT was set
                    // — the acknowledged vector is recorded in exit_intr_info.
                    if self.in_vmx_guest {
                        match self.vmexit_check_event_intr(vector) {
                            Ok(true) => {
                                self.ext = false;
                                self.prev_rip = self.rip();
                                return false;
                            }
                            Ok(false) => {}
                            Err(super::error::CpuError::CpuLoopRestart) => {
                                self.ext = false;
                                self.prev_rip = self.rip();
                                return false;
                            }
                            Err(e) => {
                                tracing::warn!("VMX ext-intr post-ack vmexit failed: {:?}", e);
                            }
                        }
                    }
                    let result = self.interrupt(vector, super::exception::InterruptType::ExternalInterrupt, false, false, 0);
                    self.ext = false;
                    delivered = true;
                    match result {
                        Ok(()) => {
                            // Bochs event.cc — update prev_rip after delivery
                            self.prev_rip = self.rip();
                        }
                        Err(super::error::CpuError::CpuLoopRestart) => {
                            // interrupt() delivered via exception path (CpuLoopRestart).
                            // Bochs event.cc: prev_rip = RIP after successful delivery.
                            self.prev_rip = self.rip();
                            return false;
                        }
                        Err(e) => {
                            tracing::warn!("LAPIC interrupt delivery failed: {:?}", e);
                        }
                    }
                }
            }

            // Then check PIC (legacy 8259 path) — only if LAPIC didn't deliver
            if !delivered {
              if let Some(pic) = pic {
                if pic.has_interrupt() {
                    let vector = pic.iac();
                    tracing::trace!("HAE: delivering PIC vector={:#04x} at RIP={:#x} CS={:#06x} mode={:?} IF={}",
                        vector, self.rip(), self.sregs[0].selector.value,
                        self.cpu_mode, self.eflags.contains(super::eflags::EFlags::IF_));
                    // Wake from halt if needed
                    self.activity_state = CpuActivityState::Active;
                    // Mark as external interrupt (EXT=1)
                    self.ext = true;
                    // Bochs vmexit.cc VMexit_Event(BX_EXTERNAL_INTERRUPT, vector,
                    // 0, 0): post-ack pin-based exit when INTA_ON_VMEXIT was set.
                    if self.in_vmx_guest {
                        match self.vmexit_check_event_intr(vector) {
                            Ok(true) => {
                                self.ext = false;
                                self.prev_rip = self.rip();
                                return false;
                            }
                            Ok(false) => {}
                            Err(super::error::CpuError::CpuLoopRestart) => {
                                self.ext = false;
                                self.prev_rip = self.rip();
                                return false;
                            }
                            Err(e) => {
                                tracing::warn!("VMX ext-intr post-ack vmexit failed: {:?}", e);
                            }
                        }
                    }
                    // Deliver interrupt (matches Bochs interrupt() call in event.cc)
                    let result = self.interrupt(vector, super::exception::InterruptType::ExternalInterrupt, false, false, 0);
                    self.ext = false;
                    match result {
                        Ok(()) => {
                            self.prev_rip = self.rip();
                        }
                        Err(super::error::CpuError::CpuLoopRestart) => {
                            self.prev_rip = self.rip();
                            return false;
                        }
                        Err(e) => {
                            tracing::warn!("PIC interrupt delivery failed: {:?}", e);
                        }
                    }
                } else {
                    #[cfg(debug_assertions)] { self.diag_hae_intr_pic_empty += 1; }
                }
            }
            }
        } else if self.pending_event
            & (Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR)
            != 0
        {
            // Event is pending but masked (IF=0) — don't clear it, just count
            #[cfg(debug_assertions)] { self.diag_hae_intr_if_blocked += 1; }
        }

        // DMA HRQ handling (Bochs event.cc)
        // NOTE: similar code in handleWaitForEvent (event.cc)
        // Assert Hold Acknowledge (HLDA) and perform DMA transfer
        if self.get_hrq() {
            if let Some(dma) = dma {
                dma.raise_hlda();
            }
        }

        // End of handleAsyncEvent: schedule TF->debug_trap for next boundary
        // Bochs event.cc
        if self.eflags.contains(EFlags::TF) {
            self.debug_trap |= Self::BX_DEBUG_SINGLE_STEP_BIT;
            self.async_event = 1;
        }

        // Bochs event.cc: Conditionally clear async_event
        // Only clear when no events remain pending (debug_trap, pending events, HRQ)
        let has_unmasked_events = (self.pending_event & !self.event_mask) != 0;
        let hrq_active = self.get_hrq();
        if !has_unmasked_events && self.debug_trap == 0 && !hrq_active {
            self.async_event = 0;
        }

        false // Continue execution
    }

    /// Handle wait for event - matches Bochs event.cc:handleWaitForEvent()
    /// Called when CPU is halted (HLT) or waiting (MWAIT)
    /// Returns true if should return from cpu_loop
    fn handle_wait_for_event(&mut self, dma: Option<&mut crate::dma::BxDmaC>) -> bool {
        // For WAIT_FOR_SIPI, just return (matches Bochs event.cc)
        if matches!(self.activity_state, CpuActivityState::WaitForSipi) {
            tracing::trace!("CPU in WAIT_FOR_SIPI state, returning from cpu_loop");
            return true;
        }

        // Handle DMA also when CPU is halted (Bochs event.cc)
        if self.get_hrq() {
            if let Some(dma) = dma {
                dma.raise_hlda();
            }
        }

        // For single processor, check if an external interrupt can wake us.
        // Matches Bochs event.cc
        //
        // MWAIT_IF (ECX[0]=1 at MWAIT): wake on interrupt even when IF=0
        // (Bochs event.cc)
        let mwait_if = matches!(self.activity_state, CpuActivityState::MwaitIf);
        let in_mwait = matches!(self.activity_state, CpuActivityState::Mwait | CpuActivityState::MwaitIf);

        // NMI can always wake from HLT (Bochs event.cc)
        if self.pending_event & Self::BX_EVENT_NMI != 0 {
            // Bochs event.cc: reset monitor when waking from MWAIT
            if in_mwait {
                self.monitor.reset_monitor();
            }
            self.activity_state = CpuActivityState::Active;
            self.inhibit_mask = 0;
            return false; // Continue to NMI delivery
        }

        // PIC interrupt can wake from HLT/MWAIT if IF=1
        if self.pending_event & Self::BX_EVENT_PENDING_INTR != 0
            && (self.eflags.contains(EFlags::IF_) || mwait_if) {
                // Bochs event.cc: reset monitor when waking from MWAIT
                if in_mwait {
                    self.monitor.reset_monitor();
                }
                self.activity_state = CpuActivityState::Active;
                self.inhibit_mask = 0;
                return false; // Continue to interrupt delivery
            }

        // LAPIC interrupt can also wake from HLT/MWAIT if IF=1
        if (self.pending_event & Self::BX_EVENT_PENDING_LAPIC_INTR != 0 || self.lapic.intr)
            && (self.eflags.contains(EFlags::IF_) || mwait_if) {
                // Bochs event.cc: reset monitor when waking from MWAIT
                if in_mwait {
                    self.monitor.reset_monitor();
                }
                self.activity_state = CpuActivityState::Active;
                self.inhibit_mask = 0;
                return false; // Continue to LAPIC interrupt delivery
            }

        // Monitor triggered by a write (wakeup_monitor set activity_state to Active)
        if matches!(self.activity_state, CpuActivityState::Active) {
            tracing::trace!("CPU activity_state became ACTIVE, waking up");
            self.inhibit_mask = 0;
            return false;
        }

        // Return from cpu_loop to allow other processing (matches single-CPU behavior)
        // In Bochs, BX_TICKN(10) advances time, then loops again
        // For our emulator, we return to allow GUI updates and device processing

        // Bochs event.cc: clear inhibit_mask when waking from HLT
        self.inhibit_mask = 0;
        true
    }

    /// Check code breakpoints at the given linear address (Bochs event.cc).
    /// Returns bitmap of matching breakpoints to OR into debug_trap.
    /// In Bochs, this checks DR0-DR3 against laddr when DR7 L/G bits enable
    /// execution breakpoints (R/W field = 0b00). Each match sets the
    /// corresponding B0-B3 bit in the returned value.
    /// Not implemented — hardware debug breakpoints (DR0-DR3 + DR7) not fully
    /// supported yet. Returns 0 (no breakpoints configured).
    fn code_breakpoint_match(&self, _laddr: u64) -> u32 {
        0
    }
}
