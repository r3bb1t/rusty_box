use super::{
    cpu::CpuActivityState,
    cpuid::BxCpuIdTrait,
    decoder::BxSegregs,
    eflags::EFlags,
    svm::{SvmVmexit, BX_VM_CR_MSR_INIT_REDIRECT_MASK, SVM_INTERCEPT0_INIT, SVM_INTERCEPT0_SMI},
    vmx::VmxVmexitReason,
    BxCpuC,
};

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'c, I, T> {
    /// Handle async events - matches Bochs event.cc handleAsyncEvent()
    /// Returns true if should return from cpu_loop
    pub(super) fn handle_async_event(
        &mut self,
        pic: Option<&mut crate::pic::BxPicC>,
        mut dma: Option<&mut crate::dma::BxDmaC>,
        mut mem: Option<&mut crate::memory::BxMemC<'c>>,
        pins: &[crate::memory::CpuTlbPin],
    ) -> bool {
        // Check if CPU is in non-active state (HLT, MWAIT, etc.)
        // Matches Bochs event.cc
        if !matches!(self.activity_state, CpuActivityState::Active) {
            // For one processor, pass the time as quickly as possible until
            // an interrupt wakes up the CPU.
            if self.handle_wait_for_event(dma.as_deref_mut(), mem.as_deref_mut(), pins) {
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

        // SMI (Bochs event.cc): gated on SVM GIF; an SVM guest with the SMI
        // intercept set exits instead (Svm_Vmexit longjmps, so the SMI stays
        // pending and GIF=0 after the exit holds it until STGI).
        if self.is_unmasked_event_pending(Self::BX_EVENT_SMI) && self.svm_gif {
            if self.in_svm_guest && self.svm_intercept_check(SVM_INTERCEPT0_SMI) {
                match self.svm_vmexit(SvmVmexit::Smi as i32, 0, 0) {
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => tracing::warn!("SVM SMI vmexit failed: {:?}", e),
                    Ok(()) => {}
                }
            }
            self.clear_event(Self::BX_EVENT_SMI);
            self.enter_system_management_mode();
        }

        // INIT (Bochs event.cc): reset CPU via reset(BX_RESET_SOFTWARE).
        // Used by multiprocessor startup (INIT-SIPI-SIPI sequence).
        // Gated on SVM GIF like SMI.
        if self.is_unmasked_event_pending(Self::BX_EVENT_INIT) && self.svm_gif {
            // Bochs event.cc: SVM INIT intercept exits with INIT still pending.
            if self.in_svm_guest && self.svm_intercept_check(SVM_INTERCEPT0_INIT) {
                match self.svm_vmexit(SvmVmexit::Init as i32, 0, 0) {
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => tracing::warn!("SVM INIT vmexit failed: {:?}", e),
                    Ok(()) => {}
                }
            }
            // Bochs event.cc: VM_CR.R_INIT redirects INIT to #SX; the only
            // error code is 1 and indicates redirection of INIT.
            if self.msr.svm_vm_cr & BX_VM_CR_MSR_INIT_REDIRECT_MASK != 0 {
                self.clear_event(Self::BX_EVENT_INIT);
                tracing::info!("SVM INIT Redirect to #SX");
                match self.exception(super::cpu::Exception::Sx, 1) {
                    Ok(()) | Err(super::error::CpuError::CpuLoopRestart) => {}
                    Err(e) => tracing::warn!("#SX INIT redirect failed: {:?}", e),
                }
                return false;
            }
            self.clear_event(Self::BX_EVENT_INIT);
            // Bochs event.cc: INIT in VMX non-root operation causes
            // VMexit(VMX_VMEXIT_INIT) — the exit unwinds (Bochs longjmp),
            // so the CPU reset below is skipped.
            if self.in_vmx_guest {
                match self.vmexit_unconditional(VmxVmexitReason::Init, 0) {
                    Ok(true) | Err(super::error::CpuError::CpuLoopRestart) => {
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => tracing::warn!("VMX INIT vmexit failed: {:?}", e),
                    Ok(false) => {}
                }
            }
            if self.bx_cpuid == 0 {
                tracing::warn!("CPU 0 INIT event delivered; software-resetting BSP");
            } else {
                tracing::debug!(
                    "CPU {} INIT event delivered; software-resetting AP",
                    self.bx_cpuid
                );
            }
            self.reset(super::ResetReason::Software);
            if !matches!(self.activity_state, CpuActivityState::Active) {
                return true;
            }
        }
        // VMX Monitor-Trap-Flag — Bochs event.cc handleAsyncEvent runs
        // this in Priority 3 (between INIT and the Priority-4 debug-trap
        // check), gated only on the event being pending; the unmasked
        // path takes the VMEXIT, the masked path simply unmasks for the
        // next boundary.
        if self.in_vmx_guest {
            match self.vmexit_check_monitor_trap_flag() {
                Ok(true) => {
                    self.prev_rip = self.rip();
                    return false;
                }
                Err(super::error::CpuError::CpuLoopRestart) => {
                    self.prev_rip = self.rip();
                    return false;
                }
                Err(e) => {
                    tracing::warn!("VMX MTF vmexit failed: {:?}", e);
                }
                Ok(false) => {}
            }
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

        // Bochs event.cc Priority 5: external interrupts. Bochs structures
        // this as a single if/else-if chain so each branch is mutually
        // exclusive — exactly one of {skip, preemption-timer VMEXIT,
        // NMI-window VMEXIT, NMI delivery, interrupt-window VMEXIT,
        // external-interrupt delivery} runs per boundary. The LAPIC poll
        // matches Bochs's `vmx_preemption_timer_expired` callback by
        // signalling BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED when the
        // absolute fire time has been reached.
        if self.in_vmx_guest {
            self.poll_vmx_preemption_timer();
        }

        // Bochs vapic.cc / event.cc — process posted interrupts at the
        // start of the Priority-5 external-event check so a pending
        // notification clears PID.ON and raises
        // BX_EVENT_PENDING_VMX_VIRTUAL_INTR before the interrupt-window
        // VMEXIT path runs.
        if self.in_vmx_guest && self.posted_interrupt_pending() {
            if let Err(e) = self.process_posted_interrupts() {
                tracing::warn!("posted-interrupt processing failed: {:?}", e);
            }
        }

        if self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS) {
            // STI/MOV SS shadow — skip all external interrupts this boundary
            // (Bochs event.cc)
        } else if self.in_vmx_guest
            && self.is_unmasked_event_pending(Self::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED)
        {
            // Bochs event.cc — VMexit(VMX_VMEXIT_VMX_PREEMPTION_TIMER_EXPIRED, 0).
            match self.vmexit_check_preemption_timer() {
                Ok(true) | Err(super::error::CpuError::CpuLoopRestart) => {
                    self.prev_rip = self.rip();
                    return false;
                }
                Err(e) => {
                    tracing::warn!("VMX preemption-timer vmexit failed: {:?}", e);
                }
                Ok(false) => {}
            }
        } else if self.in_vmx_guest
            && self.is_unmasked_event_pending(Self::BX_EVENT_VMX_VIRTUAL_NMI)
        {
            // Bochs event.cc — VMexit(VMX_VMEXIT_NMI_WINDOW, 0).
            match self.vmexit_check_nmi_window() {
                Ok(true) | Err(super::error::CpuError::CpuLoopRestart) => {
                    self.prev_rip = self.rip();
                    return false;
                }
                Err(e) => {
                    tracing::warn!("VMX NMI-window vmexit failed: {:?}", e);
                }
                Ok(false) => {}
            }
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
        } else if self.in_vmx_guest
            && (self.pending_event & Self::BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING) != 0
            && self.eflags.contains(EFlags::IF_)
        {
            // Bochs event.cc — VMexit(VMX_VMEXIT_INTERRUPT_WINDOW, 0).
            match self.vmexit_check_interrupt_window() {
                Ok(true) | Err(super::error::CpuError::CpuLoopRestart) => {
                    self.prev_rip = self.rip();
                    return false;
                }
                Err(e) => {
                    tracing::warn!("VMX interrupt-window vmexit failed: {:?}", e);
                }
                Ok(false) => {}
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
                self.sync_lapic_events();
                if vector > 0 {
                    #[cfg(debug_assertions)]
                    {
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
                    let result = self.interrupt(
                        vector,
                        super::exception::InterruptType::ExternalInterrupt,
                        false,
                        false,
                        0,
                    );
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
                        let result = self.interrupt(
                            vector,
                            super::exception::InterruptType::ExternalInterrupt,
                            false,
                            false,
                            0,
                        );
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
                        // The CPU event bit mirrors the PIC INT pin. If no vector is
                        // deliverable, reconcile a stale assertion now; otherwise
                        // async_event remains set and every instruction exits its
                        // trace to rescan an empty PIC indefinitely.
                        // Consume deferred PIC edge flags while reconciling the
                        // current deasserted INT pin. A later device assertion
                        // will set irq_pending again, so it cannot be erased by
                        // this acknowledge's stale irq_cleared flag.
                        pic.reconcile_deasserted_intr();
                        self.clear_event(Self::BX_EVENT_PENDING_INTR);
                        if self.pending_event & Self::BX_EVENT_PENDING_LAPIC_INTR == 0 {
                            self.async_event = super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
                        }
                        #[cfg(debug_assertions)]
                        {
                            self.diag_hae_intr_pic_empty += 1;
                        }
                    }
                }
            }
        } else if self.pending_event
            & (Self::BX_EVENT_PENDING_INTR | Self::BX_EVENT_PENDING_LAPIC_INTR)
            != 0
        {
            // Event is pending but masked (IF=0) — don't clear it, just count
            #[cfg(debug_assertions)]
            {
                self.diag_hae_intr_if_blocked += 1;
            }
        }

        // DMA HRQ handling (Bochs event.cc)
        // NOTE: similar code in handleWaitForEvent (event.cc)
        // Assert Hold Acknowledge (HLDA) and perform DMA transfer
        if self.get_hrq() {
            if let Some(dma) = dma {
                dma.raise_hlda(mem.as_deref_mut(), pins);
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

    /// Bochs `deliver_SMI`: signal SMI unconditionally; masking is
    /// checked when the event is processed in `handle_async_event`.
    #[inline]
    pub(crate) fn deliver_smi(&mut self) {
        self.signal_event(Self::BX_EVENT_SMI);
    }

    /// Bochs `deliver_NMI`: signal NMI.
    #[inline]
    pub(crate) fn deliver_nmi(&mut self) {
        self.signal_event(Self::BX_EVENT_NMI);
    }

    /// Bochs `deliver_INIT`: signal a software reset if INIT is unmasked.
    #[inline]
    pub(crate) fn deliver_init(&mut self) {
        if (Self::BX_EVENT_INIT & self.event_mask) == 0 {
            self.signal_event(Self::BX_EVENT_INIT);
        }
    }

    /// Bochs `deliver_SIPI`: start a CPU waiting for SIPI at `vector * 0x100`.
    pub(crate) fn deliver_sipi(&mut self, vector: u8) {
        if !matches!(self.activity_state, CpuActivityState::WaitForSipi) {
            tracing::info!(
                "CPU {} started by APIC, but was not halted at that time",
                self.bx_cpuid
            );
            return;
        }

        self.unmask_event(Self::BX_EVENT_INIT | Self::BX_EVENT_SMI | Self::BX_EVENT_NMI);
        // Bochs event.cc deliver_SIPI: SIPI arriving while in VMX non-root
        // operation (guest activity state wait-for-SIPI) causes
        // VMexit(VMX_VMEXIT_SIPI, vector) — the exit unwinds (Bochs longjmp),
        // so the real-mode activation below is skipped;
        // vmexit_load_host_state already sets ACTIVE and clears inhibits.
        // Callers that invoke this from emulator context wire the memory bus
        // first (apply_lapic_cpu_event), so the VMEXIT MSR lists resolve.
        if self.in_vmx_guest {
            self.async_event &= !Self::BX_ASYNC_EVENT_SLEEP;
            match self.vmexit_unconditional(VmxVmexitReason::Sipi, vector as u64) {
                Ok(_) | Err(super::error::CpuError::CpuLoopRestart) => {}
                Err(e) => tracing::warn!("VMX SIPI vmexit failed: {:?}", e),
            }
            return;
        }
        self.activity_state = CpuActivityState::Active;
        self.async_event &= !Self::BX_ASYNC_EVENT_SLEEP;
        self.set_rip(0);
        self.load_seg_reg_real_mode(BxSegregs::Cs, (vector as u16) << 8);
        tracing::info!(
            "CPU {} started up at {:04X}:{:08X} by APIC",
            self.bx_cpuid,
            (vector as u16) << 8,
            self.eip()
        );
    }

    /// Handle wait for event - matches Bochs event.cc:handleWaitForEvent()
    /// Called when CPU is halted (HLT) or waiting (MWAIT)
    /// Returns true if should return from cpu_loop
    fn handle_wait_for_event(
        &mut self,
        dma: Option<&mut crate::dma::BxDmaC>,
        mem: Option<&mut crate::memory::BxMemC<'c>>,
        pins: &[crate::memory::CpuTlbPin],
    ) -> bool {
        // For WAIT_FOR_SIPI, just return (matches Bochs event.cc)
        if matches!(self.activity_state, CpuActivityState::WaitForSipi) {
            tracing::trace!("CPU in WAIT_FOR_SIPI state, returning from cpu_loop");
            return true;
        }

        // Handle DMA also when CPU is halted (Bochs event.cc)
        if self.get_hrq() {
            if let Some(dma) = dma {
                dma.raise_hlda(mem, pins);
            }
        }

        // For single processor, check if an external interrupt can wake us.
        // Matches Bochs event.cc
        //
        // MWAIT_IF (ECX[0]=1 at MWAIT): wake on interrupt even when IF=0
        // (Bochs event.cc)
        let mwait_if = matches!(self.activity_state, CpuActivityState::MwaitIf);
        let in_mwait = matches!(
            self.activity_state,
            CpuActivityState::Mwait | CpuActivityState::MwaitIf
        );

        // SMI/INIT wake HLT/MWAIT regardless of IF (Bochs event.cc
        // handleWaitForEvent checks unmasked BX_EVENT_SMI | BX_EVENT_INIT).
        if self.is_unmasked_event_pending(Self::BX_EVENT_SMI | Self::BX_EVENT_INIT) {
            if in_mwait {
                self.monitor.reset_monitor();
            }
            self.activity_state = CpuActivityState::Active;
            self.inhibit_mask = 0;
            return false; // Continue to SMI/INIT delivery
        }

        // NMI can wake from HLT/MWAIT only when unmasked (Bochs event.cc).
        if self.is_unmasked_event_pending(Self::BX_EVENT_NMI) {
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
            && (self.eflags.contains(EFlags::IF_) || mwait_if)
        {
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
            && (self.eflags.contains(EFlags::IF_) || mwait_if)
        {
            // Bochs event.cc: reset monitor when waking from MWAIT
            if in_mwait {
                self.monitor.reset_monitor();
            }
            self.activity_state = CpuActivityState::Active;
            self.inhibit_mask = 0;
            return false; // Continue to LAPIC interrupt delivery
        }

        // Monitor triggered by a write (wakeup_monitor set activity_state to
        // Active). Bochs event.cc breaks out here without touching
        // inhibit_mask — only the interrupt-wake branch clears it.
        if matches!(self.activity_state, CpuActivityState::Active) {
            tracing::trace!("CPU activity_state became ACTIVE, waking up");
            return false;
        }

        // HALT condition remains: return from cpu_loop so other CPUs (or the
        // emulator scheduler) get a chance, leaving inhibit_mask untouched
        // exactly like Bochs event.cc handleWaitForEvent's return-1 path.
        true
    }

    /// Check code (instruction-execution) breakpoints at `laddr`.
    /// Bochs crregs.cc `code_breakpoint_match`.
    fn code_breakpoint_match(&self, laddr: u64) -> u32 {
        // RF suppresses instruction breakpoints for exactly one instruction.
        if self.eflags.contains(EFlags::RF) {
            return 0;
        }
        if self.dr7.bp_enabled() != 0 {
            return self.hwdebug_compare(
                laddr,
                1,
                Self::BX_HW_DEBUG_INSTRUCTION,
                Self::BX_HW_DEBUG_INSTRUCTION,
            );
        }
        0
    }

    /// Compare a linear-address range against DR0-DR3 under DR7.
    /// Bochs crregs.cc `hwdebug_compare`. `opa`/`opb` are the accepted DR7
    /// R/W field values (instruction, memory-write, or memory-read/write).
    /// Returns the DR6 status bits to OR into `debug_trap`: B0-B3 for each
    /// matching register, plus `BX_DEBUG_TRAP_HIT` if any matching register
    /// was actually enabled in DR7.
    fn hwdebug_compare(&self, laddr_0: u64, size: u64, opa: u32, opb: u32) -> u32 {
        // Indexed by the 2-bit LEN field: 00b=1, 01b=2, 10b=undef(8), 11b=4.
        const ALIGNMENT_MASK: [u64; 4] = [0x0, 0x1, 0x7, 0x3];

        let dr7 = self.dr7.get32();
        let laddr_n = laddr_0 + (size - 1);

        let dr_len = [
            self.dr7.len0(),
            self.dr7.len1(),
            self.dr7.len2(),
            self.dr7.len3(),
        ];
        let dr_op = [
            self.dr7.r_w0(),
            self.dr7.r_w1(),
            self.dr7.r_w2(),
            self.dr7.r_w3(),
        ];

        let mut dr6_mask = 0u32;
        for n in 0..4 {
            let mask = ALIGNMENT_MASK[dr_len[n] as usize];
            let dr_start = self.dr[n] & !mask;
            let dr_end = dr_start + mask;

            if (dr_op[n] == opa || dr_op[n] == opb) && laddr_0 <= dr_end && laddr_n >= dr_start {
                dr6_mask |= 1 << n;
                // Report HIT only if this breakpoint was enabled (L/G pair).
                if dr7 & (3 << (n * 2)) != 0 {
                    dr6_mask |= Self::BX_DEBUG_TRAP_HIT;
                }
            }
        }

        dr6_mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::ResetReason;
    use crate::params::{BxParams, CpuTopology};

    const IA32_APIC_BASE_BSP_FLAG: u64 = 0x100;
    const TEST_SIPI_VECTOR: u8 = 0x08;
    const TEST_SIPI_CS_SELECTOR: u16 = (TEST_SIPI_VECTOR as u16) << 8;
    const TEST_SIPI_CS_BASE: u64 = (TEST_SIPI_CS_SELECTOR as u64) << 4;
    const NONZERO_RAX_SENTINEL: u64 = 0xFEED_BEEF;
    const TEST_SMP_PACKAGES: u32 = 2;
    const TEST_SMP_CORES: u32 = 1;
    const TEST_SMP_THREADS: u32 = 1;

    fn smp_topology() -> CpuTopology {
        BxParams::default()
            .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
            .unwrap()
            .cpu_topology()
    }

    fn make_cpu(cpu_id: u32) -> alloc::boxed::Box<BxCpuC<'static, Corei7SkylakeX>> {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.configure_smp(cpu_id, smp_topology());
        cpu
    }

    #[test]
    fn hardware_reset_puts_application_processor_in_wait_for_sipi() {
        let mut bsp = make_cpu(0);
        let mut ap = make_cpu(1);

        bsp.reset(ResetReason::Hardware);
        ap.reset(ResetReason::Hardware);

        assert_eq!(bsp.activity_state, CpuActivityState::Active);
        assert_ne!(
            bsp.msr.apicbase & IA32_APIC_BASE_BSP_FLAG,
            0,
            "BSP bit must be set on CPU 0"
        );
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);
        assert_eq!(
            ap.msr.apicbase & IA32_APIC_BASE_BSP_FLAG,
            0,
            "AP must not advertise BSP bit"
        );
        assert_ne!(
            ap.async_event & BxCpuC::<Corei7SkylakeX>::BX_ASYNC_EVENT_SLEEP,
            0
        );
    }

    #[test]
    fn sipi_starts_only_waiting_application_processor_at_vector_segment() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);

        ap.deliver_sipi(TEST_SIPI_VECTOR);

        assert_eq!(ap.activity_state, CpuActivityState::Active);
        assert_eq!(ap.get_cs_selector(), TEST_SIPI_CS_SELECTOR);
        assert_eq!(ap.get_cs_base(), TEST_SIPI_CS_BASE);
        assert_eq!(ap.rip(), 0);
    }

    #[test]
    fn init_event_software_resets_active_ap_back_to_wait_for_sipi() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        ap.deliver_sipi(TEST_SIPI_VECTOR);
        ap.set_rax(NONZERO_RAX_SENTINEL);

        ap.deliver_init();
        assert_ne!(
            ap.pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT,
            0
        );
        let exited = ap.handle_async_event(None, None, None, &[]);

        assert!(exited, "AP entering WAIT_FOR_SIPI must exit the cpu loop");
        assert_eq!(ap.rax(), 0);
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);
    }

    #[test]
    fn svm_gif_false_holds_smi_and_init_pending() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        ap.deliver_sipi(TEST_SIPI_VECTOR);
        ap.set_rax(NONZERO_RAX_SENTINEL);

        // Bochs event.cc handleAsyncEvent: SMI and INIT checks are gated on
        // SVM_GIF; with GIF clear both stay pending and nothing happens.
        ap.svm_gif = false;
        ap.deliver_smi();
        ap.deliver_init();
        let exited = ap.handle_async_event(None, None, None, &[]);

        assert!(!exited);
        assert!(ap.is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_SMI));
        assert!(ap.is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT));
        assert!(!ap.in_smm, "SMI must not enter SMM while GIF=0");
        assert_eq!(ap.activity_state, CpuActivityState::Active);
        assert_eq!(
            ap.rax(),
            NONZERO_RAX_SENTINEL,
            "INIT must not reset while GIF=0"
        );

        // STGI: with GIF set again, the held INIT is processed. Drop the SMI
        // first so this test does not depend on SMM entry machinery.
        ap.clear_event(BxCpuC::<Corei7SkylakeX>::BX_EVENT_SMI);
        ap.svm_gif = true;
        let exited = ap.handle_async_event(None, None, None, &[]);

        assert!(exited, "AP entering WAIT_FOR_SIPI must exit the cpu loop");
        assert_eq!(ap.rax(), 0);
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);
    }

    #[test]
    fn svm_smi_intercept_takes_vmexit_and_keeps_smi_pending() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        ap.deliver_sipi(TEST_SIPI_VECTOR);

        ap.in_svm_guest = true;
        let mut vmcb = crate::cpu::svm::VmcbCache::default();
        vmcb.ctrls.intercept_vector[0] |= 1 << crate::cpu::svm::SVM_INTERCEPT0_SMI;
        ap.vmcb = Some(vmcb);

        ap.deliver_smi();
        let exited = ap.handle_async_event(None, None, None, &[]);

        // Bochs event.cc: Svm_Vmexit(SVM_VMEXIT_SMI) fires instead of SMM
        // entry, and the SMI stays pending (held by GIF=0 after the exit).
        assert!(!exited);
        assert!(!ap.in_svm_guest, "SMI intercept must exit SVM guest mode");
        assert!(!ap.svm_gif, "GIF must be clear after SVM VMEXIT");
        assert!(
            ap.pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_SMI != 0,
            "intercepted SMI must stay pending"
        );
        assert!(!ap.in_smm, "SMI intercept must preempt SMM entry");
    }

    #[test]
    fn svm_init_intercept_takes_vmexit_and_keeps_init_pending() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        ap.deliver_sipi(TEST_SIPI_VECTOR);

        ap.in_svm_guest = true;
        let mut vmcb = crate::cpu::svm::VmcbCache::default();
        vmcb.ctrls.intercept_vector[0] |= 1 << crate::cpu::svm::SVM_INTERCEPT0_INIT;
        ap.vmcb = Some(vmcb);

        ap.deliver_init();
        let exited = ap.handle_async_event(None, None, None, &[]);

        // Bochs event.cc: Svm_Vmexit(SVM_VMEXIT_INIT) fires with INIT still
        // pending; the CPU reset is skipped.
        assert!(!exited);
        assert!(!ap.in_svm_guest, "INIT intercept must exit SVM guest mode");
        assert!(!ap.svm_gif, "GIF must be clear after SVM VMEXIT");
        assert!(
            ap.pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT != 0,
            "intercepted INIT must stay pending"
        );
        // An INIT reset would park the AP in WAIT_FOR_SIPI; the intercept
        // must preempt it. (RAX is not usable as a reset probe here: SVM
        // VMEXIT legitimately restores host RAX from the VMCB host state.)
        assert_eq!(
            ap.activity_state,
            CpuActivityState::Active,
            "INIT intercept must preempt the CPU reset"
        );
    }

    #[test]
    fn code_breakpoint_matches_enabled_instruction_register() {
        use crate::cpu::crregs::BxDr7;

        let mut cpu = make_cpu(0);
        cpu.reset(ResetReason::Hardware);

        // DR0 = target laddr; DR7: L0=1 (bit 0), R/W0=00b (instruction),
        // LEN0=00b (1 byte).
        cpu.dr[0] = 0x1234;
        cpu.dr7 = BxDr7::from_bits_retain(0x1);
        cpu.eflags.remove(crate::cpu::eflags::EFlags::RF);

        let bits = cpu.code_breakpoint_match(0x1234);
        assert_ne!(bits & 0x1, 0, "B0 status bit must be set on a match");
        assert_ne!(
            bits & BxCpuC::<Corei7SkylakeX>::BX_DEBUG_TRAP_HIT,
            0,
            "HIT must be set because DR0 is enabled in DR7"
        );

        // A different address does not match.
        assert_eq!(cpu.code_breakpoint_match(0x5678), 0);

        // RF suppresses instruction breakpoints for one instruction.
        cpu.eflags.insert(crate::cpu::eflags::EFlags::RF);
        assert_eq!(cpu.code_breakpoint_match(0x1234), 0);
    }

    #[test]
    fn disabled_instruction_register_matches_without_hit() {
        use crate::cpu::crregs::BxDr7;

        let mut cpu = make_cpu(0);
        cpu.reset(ResetReason::Hardware);

        // DR0 armed at the address but with a *different* enabled breakpoint
        // (L1 for DR1) — Bochs still sets B0 (status), but HIT only when the
        // matching register itself is enabled.
        cpu.dr[0] = 0x2000;
        cpu.dr[1] = 0x9999;
        cpu.dr7 = BxDr7::from_bits_retain(1 << 2); // L1 enabled, L0 disabled
        cpu.eflags.remove(crate::cpu::eflags::EFlags::RF);

        let bits = cpu.code_breakpoint_match(0x2000);
        assert_ne!(bits & 0x1, 0, "B0 status bit still reported for a match");
        assert_eq!(
            bits & BxCpuC::<Corei7SkylakeX>::BX_DEBUG_TRAP_HIT,
            0,
            "HIT must NOT be set: DR0 is not enabled in DR7"
        );
    }

    #[test]
    fn smm_entry_masks_smi_and_nmi_until_rsm() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        ap.deliver_sipi(TEST_SIPI_VECTOR);

        let held = BxCpuC::<Corei7SkylakeX>::BX_EVENT_SMI
            | BxCpuC::<Corei7SkylakeX>::BX_EVENT_NMI
            | BxCpuC::<Corei7SkylakeX>::BX_EVENT_VMX_VIRTUAL_NMI;

        // SMI delivery enters SMM at the next instruction boundary.
        ap.deliver_smi();
        let exited = ap.handle_async_event(None, None, None, &[]);
        assert!(!exited);
        assert!(ap.in_smm, "SMI must enter System Management Mode");
        // Bochs smm.cc enter_system_management_mode masks SMI/NMI/virtual-NMI.
        assert_eq!(
            ap.event_mask & held,
            held,
            "SMM entry must mask SMI, NMI, and VMX virtual-NMI"
        );

        // An NMI arriving during SMM stays pending and is not dispatched.
        ap.deliver_nmi();
        let rip_in_smm = ap.rip();
        let exited = ap.handle_async_event(None, None, None, &[]);
        assert!(!exited);
        assert_ne!(
            ap.pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_NMI,
            0,
            "NMI during SMM must stay pending until RSM"
        );
        assert_eq!(
            ap.rip(),
            rip_in_smm,
            "a masked NMI must not be dispatched inside SMM"
        );

        // RSM releases the held events (Bochs smm.cc RSM).
        ap.rsm(&crate::cpu::decoder::Instruction::default())
            .expect("RSM must succeed outside VMX/SVM guest mode");
        assert!(!ap.in_smm);
        assert_eq!(
            ap.event_mask & held,
            0,
            "RSM must unmask SMI, NMI, and VMX virtual-NMI"
        );
        assert_ne!(
            ap.pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_NMI,
            0,
            "the held NMI is still pending after RSM for the next boundary"
        );
    }

    #[test]
    fn smi_parks_vmx_mode_and_rsm_restores_it() {
        use crate::cpu::crregs::{BxCr0, BxCr4};

        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        ap.deliver_sipi(TEST_SIPI_VECTOR);

        // Simulate a CPU in VMX non-root operation when the SMI hits.
        ap.cr4.insert(BxCr4::VMXE);
        ap.in_vmx = true;
        ap.in_vmx_guest = true;

        ap.deliver_smi();
        let exited = ap.handle_async_event(None, None, None, &[]);
        assert!(!exited);
        assert!(ap.in_smm, "SMI must enter System Management Mode");

        // Bochs smm.cc enter_system_management_mode: VMX operation is left
        // and parked for the duration of SMM.
        assert!(!ap.in_vmx, "SMM entry must leave VMX operation");
        assert!(!ap.in_vmx_guest, "SMM entry must leave VMX non-root mode");
        assert!(ap.in_smm_vmx, "SMM entry must park the in_vmx flag");
        assert!(
            ap.in_smm_vmx_guest,
            "SMM entry must park the in_vmx_guest flag"
        );
        assert!(
            !ap.cr4.contains(BxCr4::VMXE),
            "SMM entry must clear CR4.VMXE"
        );

        // RSM restores VMX operation and forces CR0.PE/NE/PG + CR4.VMXE
        // in the restored state (Bochs smm.cc
        // resume_from_system_management_mode).
        ap.rsm(&crate::cpu::decoder::Instruction::default())
            .expect("RSM must succeed outside VMX/SVM guest mode");
        assert!(!ap.in_smm);
        assert!(ap.in_vmx, "RSM must restore VMX root operation");
        assert!(ap.in_vmx_guest, "RSM must restore VMX non-root mode");
        assert!(
            ap.cr4.contains(BxCr4::VMXE),
            "RSM into VMX operation must force CR4.VMXE"
        );
        let forced_cr0 = (BxCr0::PG | BxCr0::NE | BxCr0::PE).bits();
        assert_eq!(
            ap.cr0.get32() & forced_cr0,
            forced_cr0,
            "RSM into VMX operation must force CR0.PE, CR0.NE, and CR0.PG"
        );
    }

    #[test]
    fn vmx_sipi_takes_vmexit_instead_of_starting_ap() {
        let mut ap = make_cpu(1);
        ap.reset(ResetReason::Hardware);
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);

        // VMENTRY with guest activity state 3 leaves the CPU in
        // WAIT_FOR_SIPI while in VMX non-root operation.
        ap.in_vmx = true;
        ap.in_vmx_guest = true;

        ap.deliver_sipi(TEST_SIPI_VECTOR);

        // Bochs event.cc deliver_SIPI: VMexit(VMX_VMEXIT_SIPI, vector) fires
        // instead of the real-mode activation.
        assert!(!ap.in_vmx_guest, "SIPI must exit VMX non-root operation");
        assert_eq!(ap.vmcs.exit_reason, VmxVmexitReason::Sipi as u32);
        assert_eq!(ap.vmcs.exit_qualification, TEST_SIPI_VECTOR as u64);
        assert_ne!(
            ap.get_cs_selector(),
            TEST_SIPI_CS_SELECTOR,
            "SIPI VMexit must not load the startup CS"
        );
        assert_eq!(
            ap.event_mask
                & (BxCpuC::<Corei7SkylakeX>::BX_EVENT_SMI | BxCpuC::<Corei7SkylakeX>::BX_EVENT_NMI),
            0,
            "SIPI unmasks SMI/NMI before the VMexit (Bochs deliver_SIPI)"
        );
        assert_ne!(
            ap.event_mask & BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT,
            0,
            "the VMexit itself re-masks INIT (Bochs vmx.cc: INIT is \
             disabled in VMX root mode)"
        );
    }
}
