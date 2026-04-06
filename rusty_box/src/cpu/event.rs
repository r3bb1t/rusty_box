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
        // Bochs event.cc:246-248 — deliver #DB BEFORE clearing the bit
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

        // Priority 3: External Hardware Interventions (Bochs event.cc:250-296)
        //   FLUSH, STOPCLK, SMI, INIT

        // SMI (Bochs event.cc:255-264): enter System Management Mode.
        // Not implemented — single-CPU DLX/Alpine don't trigger SMI.
        // Bochs: clear_event(BX_EVENT_SMI); enter_system_management_mode();
        if self.is_unmasked_event_pending(Self::BX_EVENT_SMI) {
            self.clear_event(Self::BX_EVENT_SMI);
            tracing::debug!("SMI event cleared (SMM not implemented)");
        }

        // INIT (Bochs event.cc:266-296): reset CPU via reset(BX_RESET_SOFTWARE).
        // Used by multiprocessor startup (INIT-SIPI-SIPI sequence).
        // Not implemented — single-CPU emulation only.
        // Bochs: clear_event(BX_EVENT_INIT); reset(BX_RESET_SOFTWARE);
        if self.is_unmasked_event_pending(Self::BX_EVENT_INIT) {
            self.clear_event(Self::BX_EVENT_INIT);
            tracing::debug!("INIT event cleared (SMP not implemented)");
        }

        // Priority 4: Debug trap exceptions (TF single-step, data/I/O breakpoints)
        // Bochs event.cc:312-324 — check inhibition FIRST, then debug_trap
        if !self.interrupts_inhibited(Self::BX_INHIBIT_DEBUG) {
            // Bochs event.cc:316: OR code breakpoint matches into debug_trap
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

        // Priority 5: External interrupts (Bochs event.cc:326-395)
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
        if self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS) {
            // STI/MOV SS shadow — skip all external interrupts this boundary
            // (Bochs event.cc:330-341)
        } else if self.is_unmasked_event_pending(Self::BX_EVENT_NMI) {
            // NMI delivery (Bochs event.cc:352-366)
            self.clear_event(Self::BX_EVENT_NMI);
            self.mask_event(Self::BX_EVENT_NMI); // Block further NMIs until IRET
            self.activity_state = CpuActivityState::Active;
            self.ext = true;
            let result = self.interrupt(2, false, false, 0); // NMI vector = 2
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
            // HandleExtInterrupt (Bochs event.cc:373-395)
            // Deliver exactly ONE interrupt: LAPIC first, then PIC.
            let mut delivered = false;

            // Check LAPIC first (higher priority than PIC in APIC mode)
            if !delivered && self.lapic.intr {
                // Clear event before acknowledge — acknowledge_int() calls
                // service_local_apic() which may re-signal if more IRQs pending.
                self.clear_event(Self::BX_EVENT_PENDING_LAPIC_INTR);
                let vector = self.lapic.acknowledge_int();
                if vector > 0 {
                    self.diag_hae_intr_delivered += 1;
                    self.diag_iac_vectors[vector as usize] += 1;
                    self.activity_state = CpuActivityState::Active;
                    self.ext = true;
                    let result = self.interrupt(vector, false, false, 0);
                    self.ext = false;
                    delivered = true;
                    match result {
                        Ok(()) => {
                            // Bochs event.cc:183 — update prev_rip after delivery
                            self.prev_rip = self.rip();
                        }
                        Err(super::error::CpuError::CpuLoopRestart) => {
                            // interrupt() delivered via exception path (CpuLoopRestart).
                            // Bochs event.cc:183: prev_rip = RIP after successful delivery.
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
            if !delivered && self.pic_ptr.is_some() {
                // SAFETY: pic_ptr set for duration of cpu_loop_n_with_io; single-threaded access
                let pic = unsafe { self.pic_ptr.unwrap().as_mut() };
                if pic.has_interrupt() {
                    let vector = pic.iac();
                    tracing::debug!("HAE: delivering PIC vector={:#04x} at RIP={:#x} CS={:#06x} mode={:?} IF={}",
                        vector, self.rip(), self.sregs[0].selector.value,
                        self.cpu_mode, self.eflags.contains(super::eflags::EFlags::IF_));
                    // Wake from halt if needed
                    self.activity_state = CpuActivityState::Active;
                    // Mark as external interrupt (EXT=1)
                    self.ext = true;
                    // Deliver interrupt (matches Bochs interrupt() call in event.cc:389)
                    let result = self.interrupt(vector, false, false, 0);
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
                    self.diag_hae_intr_pic_empty += 1;
                }
            }
        } else if self.pending_event
            & (Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR)
            != 0
        {
            // Event is pending but masked (IF=0) — don't clear it, just count
            self.diag_hae_intr_if_blocked += 1;
        }

        // DMA HRQ handling (Bochs event.cc:390-393)
        // NOTE: similar code in handleWaitForEvent (event.cc:83-86)
        // Assert Hold Acknowledge (HLDA) and perform DMA transfer
        if self.get_hrq()
            && self.dma_ptr.is_some() {
                // SAFETY: dma_ptr set for duration of cpu_loop_n_with_io; single-threaded access
                let dma = unsafe { self.dma_ptr.unwrap().as_mut() };
                dma.raise_hlda();
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
        let hrq_active = self.get_hrq();
        if !has_unmasked_events && self.debug_trap == 0 && !hrq_active {
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

        // Handle DMA also when CPU is halted (Bochs event.cc:83-86)
        if self.get_hrq()
            && self.dma_ptr.is_some() {
                // SAFETY: dma_ptr set for duration of cpu_loop_n_with_io; single-threaded access
                let dma = unsafe { self.dma_ptr.unwrap().as_mut() };
                dma.raise_hlda();
            }

        // For single processor, check if an external interrupt can wake us.
        // Matches Bochs event.cc:52-113
        //
        // MWAIT_IF (ECX[0]=1 at MWAIT): wake on interrupt even when IF=0
        // (Bochs event.cc:73-80)
        let mwait_if = matches!(self.activity_state, CpuActivityState::MwaitIf);
        let in_mwait = matches!(self.activity_state, CpuActivityState::Mwait | CpuActivityState::MwaitIf);

        // NMI can always wake from HLT (Bochs event.cc:54-60)
        if self.pending_event & Self::BX_EVENT_NMI != 0 {
            // Bochs event.cc:63-66: reset monitor when waking from MWAIT
            #[cfg(feature = "bx_support_monitor_mwait")]
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
                // Bochs event.cc:63-66: reset monitor when waking from MWAIT
                #[cfg(feature = "bx_support_monitor_mwait")]
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
                // Bochs event.cc:63-66: reset monitor when waking from MWAIT
                #[cfg(feature = "bx_support_monitor_mwait")]
                if in_mwait {
                    self.monitor.reset_monitor();
                }
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

        // Bochs event.cc:68: clear inhibit_mask when waking from HLT
        self.inhibit_mask = 0;
        true
    }

    /// Check code breakpoints at the given linear address (Bochs event.cc:316).
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
