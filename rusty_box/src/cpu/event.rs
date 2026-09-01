use super::{
    cpu::CpuActivityState,
    decoder::BxSegregs,
    eflags::EFlags,
    exec_ctx::ExecCtx,
    svm::{SvmVmexit, BX_VM_CR_MSR_INIT_REDIRECT_MASK, SVM_INTERCEPT0_INIT, SVM_INTERCEPT0_SMI},
    vmx::VmxVmexitReason,
    BxCpuC,
};

/// What one INTA moment produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "an acknowledged vector was consumed from its controller; dropping it loses the interrupt"]
pub(crate) enum AcknowledgedInterrupt {
    /// The LAPIC answered; delivery (or a VMX fold) is the caller's.
    Lapic(u8),
    /// The 8259 answered through the fabric's counted INTA; spurious
    /// vectors arrive here exactly as a real INTA would produce them.
    Pic(u8),
    /// Nothing deliverable; the deasserted pin was reconciled.
    None,
}

/// Async-event servicing runs on the execution context: interrupt delivery
/// reads the IDT and pushes stack frames, and the hold-acknowledge path hands
/// guest memory to the DMA controller. Bochs `handleAsyncEvent` (event.cc)
/// reaches the same state through the globals this borrow replaces.
impl<T: crate::cpu::instrumentation::Instrumentation> ExecCtx<'_, T> {
    /// Handle wait for event - matches Bochs event.cc:handleWaitForEvent()
    /// Called when CPU is halted (HLT) or waiting (MWAIT)
    /// Returns true if should return from cpu_loop
    fn handle_wait_for_event(&mut self) -> bool {
        // For WAIT_FOR_SIPI, just return (matches Bochs event.cc)
        if matches!(self.activity_state, CpuActivityState::WaitForSipi) {
            tracing::trace!("CPU in WAIT_FOR_SIPI state, returning from cpu_loop");
            return true;
        }

        // Handle DMA also when CPU is halted (Bochs event.cc)
        if self.get_hrq() {
            self.device_manager.dma.raise_hlda(&mut *self.memory);
            // Bochs dma.cc raise_HLDA: synchronous set_HRQ(0) at
            // terminal count (see handle_async_event above).
            if let Some(level) = self.device_manager.dma.take_hrq_request() {
                self.pc_system.set_hrq(level);
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

        // The interrupt-class events, which need EFLAGS.IF — or the MWAIT_IF
        // state that MWAIT's ECX[0] asks for. Bochs event.cc tests these with
        // `is_pending`, not the unmasked form. `lapic.intr` is this port's
        // mirror of the LAPIC's INTR line and is read alongside the event bit
        // so a raise that has not been synced yet still ends the wait.
        let interrupt_pending = self.pending_event
            & (BxCpuC::<T>::BX_EVENT_PENDING_INTR
                | BxCpuC::<T>::BX_EVENT_PENDING_LAPIC_INTR
                | BxCpuC::<T>::BX_EVENT_PENDING_UINTR)
            != 0
            || self.lapic.intr;

        // Everything else that ends the wait: delivered asynchronously and
        // therefore independent of IF, but only while unmasked. The VMX
        // members are events whose only answer is a VMEXIT, and
        // `handle_async_event` returns the moment this function says "still
        // halted" — so an event left out here is one the processor can never
        // reach, not merely one it reaches late.
        let wake_on_unmasked: u32 = BxCpuC::<T>::BX_EVENT_NMI
            | BxCpuC::<T>::BX_EVENT_SMI
            | BxCpuC::<T>::BX_EVENT_INIT
            | BxCpuC::<T>::BX_EVENT_VMX_VTPR_UPDATE
            | BxCpuC::<T>::BX_EVENT_VMX_VEOI_UPDATE
            | BxCpuC::<T>::BX_EVENT_VMX_VIRTUAL_APIC_WRITE
            | BxCpuC::<T>::BX_EVENT_VMX_MONITOR_TRAP_FLAG
            | BxCpuC::<T>::BX_EVENT_VMX_VIRTUAL_NMI;

        if (interrupt_pending && (self.eflags.contains(EFlags::IF_) || mwait_if))
            || self.is_unmasked_event_pending(wake_on_unmasked)
        {
            // Bochs event.cc: reset the monitor when waking out of MWAIT, and
            // clear the inhibits so the resumed instruction stream starts
            // clean.
            if in_mwait {
                self.monitor.reset_monitor();
            }
            self.inhibit_mask = 0;
            return false; // Continue to delivery
        }

        // Bochs event.cc gives the expired preemption timer a branch of its
        // own, and deliberately not the two side effects above: nothing was
        // delivered, so the MONITOR stays armed and the inhibits stand. The
        // wait ends only so the VMEXIT can be taken.
        if self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED) {
            return false;
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

    /// Handle async events - matches Bochs event.cc handleAsyncEvent()
    /// Returns true if should return from cpu_loop
    pub(super) fn handle_async_event(&mut self) -> bool {
        // Check if CPU is in non-active state (HLT, MWAIT, etc.)
        // Matches Bochs event.cc
        if !matches!(self.activity_state, CpuActivityState::Active) {
            // For one processor, pass the time as quickly as possible until
            // an interrupt wakes up the CPU.
            if self.handle_wait_for_event() {
                return true; // Return to caller of cpu_loop
            }
        }

        // Priority 2: Trap on Task Switch (T flag in TSS)
        // Bochs event.cc — deliver #DB BEFORE clearing the bit
        // so that DR6 still has BT set when the handler reads it
        if self.debug_trap & BxCpuC::<T>::BX_DEBUG_TRAP_TASK_SWITCH_BIT != 0 {
            // Bochs: exception() calls longjmp, never returns.
            // We must propagate CpuLoopRestart by returning false.
            // The caller (cpu_loop_n) will restart the loop.
            if let Err(super::error::CpuError::CpuLoopRestart) =
                self.exception(super::cpu::Exception::Db, 0)
            {
                self.debug_trap &= !BxCpuC::<T>::BX_DEBUG_TRAP_TASK_SWITCH_BIT;
                return false;
            }
            self.debug_trap &= !BxCpuC::<T>::BX_DEBUG_TRAP_TASK_SWITCH_BIT;
        }

        // Priority 3: External Hardware Interventions (Bochs event.cc)
        //   FLUSH, STOPCLK, SMI, INIT

        // SMI (Bochs event.cc): gated on SVM GIF; an SVM guest with the SMI
        // intercept set exits instead (Svm_Vmexit longjmps, so the SMI stays
        // pending and GIF=0 after the exit holds it until STGI).
        if self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_SMI) && self.svm_gif {
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
            self.clear_event(BxCpuC::<T>::BX_EVENT_SMI);
            self.enter_system_management_mode();
        }

        // INIT (Bochs event.cc): reset CPU via reset(BX_RESET_SOFTWARE).
        // Used by multiprocessor startup (INIT-SIPI-SIPI sequence).
        // Gated on SVM GIF like SMI.
        if self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_INIT) && self.svm_gif {
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
                self.clear_event(BxCpuC::<T>::BX_EVENT_INIT);
                tracing::info!("SVM INIT Redirect to #SX");
                match self.exception(super::cpu::Exception::Sx, 1) {
                    Ok(()) | Err(super::error::CpuError::CpuLoopRestart) => {}
                    Err(e) => tracing::warn!("#SX INIT redirect failed: {:?}", e),
                }
                return false;
            }
            self.clear_event(BxCpuC::<T>::BX_EVENT_INIT);
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
        if !self.interrupts_inhibited(BxCpuC::<T>::BX_INHIBIT_DEBUG) {
            // Bochs event.cc: OR code breakpoint matches into debug_trap
            self.debug_trap |= self.pending_code_breakpoint_trap();
            if self.debug_trap & 0xF000 != 0 {
                // BX_DEBUG_SINGLE_STEP_BIT or BX_DEBUG_DR_ACCESS_BIT set
                // Bochs: exception() longjmps — propagate restart
                if let Err(super::error::CpuError::CpuLoopRestart) =
                    self.exception(super::cpu::Exception::Db, 0)
                {
                    // Bochs's longjmp lands in cpu_loop's setjmp handler,
                    // which commits `prev_rip = RIP` before resuming (cpu.cc).
                    // Every other delivery arm here does the same; without it
                    // the #DB handler runs with prev_rip still pointing at the
                    // interrupted instruction, so the next fault inside the
                    // handler reports the wrong address.
                    self.prev_rip = self.rip();
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

        if self.interrupts_inhibited(BxCpuC::<T>::BX_INHIBIT_INTERRUPTS) || !self.svm_gif {
            // STI/MOV SS shadow, or SVM CLGI — the whole chain is skipped this
            // boundary. Bochs event.cc heads Priority 5 with
            // `interrupts_inhibited(BX_INHIBIT_INTERRUPTS) || !SVM_GIF`, so a
            // guest running with GIF clear takes no external event at all.
        } else if self.in_vmx_guest
            && self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED)
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
            && self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_VMX_VIRTUAL_NMI)
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
        } else if self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_NMI) {
            // NMI delivery (Bochs event.cc)
            self.clear_event(BxCpuC::<T>::BX_EVENT_NMI);
            self.ext = true;
            // Bochs vmexit.cc VMexit_Event(BX_NMI, 2, 0, 0): pin-based NMI
            // exit fires before delivery into the guest IDT.
            if self.in_vmx_guest {
                match self.vmexit_check_nmi() {
                    Ok(true) => {
                        self.ext = false;
                        self.mask_event(BxCpuC::<T>::BX_EVENT_NMI);
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Ok(false) => {}
                    Err(super::error::CpuError::CpuLoopRestart) => {
                        self.ext = false;
                        self.mask_event(BxCpuC::<T>::BX_EVENT_NMI);
                        self.prev_rip = self.rip();
                        return false;
                    }
                    Err(e) => {
                        tracing::warn!("VMX NMI vmexit failed: {:?}", e);
                    }
                }
            }
            self.mask_event(BxCpuC::<T>::BX_EVENT_NMI); // Block further NMIs until IRET
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
            && (self.pending_event & BxCpuC::<T>::BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING) != 0
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
            BxCpuC::<T>::BX_EVENT_PENDING_INTR
                | BxCpuC::<T>::BX_EVENT_PENDING_LAPIC_INTR
                | BxCpuC::<T>::BX_EVENT_PENDING_VMX_VIRTUAL_INTR,
        ) {
            // HandleExtInterrupt (Bochs event.cc).
            //
            // A virtual interrupt outranks everything else here: Bochs tests
            // it with `is_pending` before `VMexit_ExtInterrupt`, delivers it
            // straight into the guest's own IDT with no exit, and longjmps
            // back to the decode loop — so nothing below runs for it.
            if self.in_vmx_guest
                && self.pending_event & BxCpuC::<T>::BX_EVENT_PENDING_VMX_VIRTUAL_INTR != 0
            {
                match self.vmx_deliver_virtual_interrupt() {
                    Ok(()) | Err(super::error::CpuError::CpuLoopRestart) => {}
                    Err(e) => {
                        tracing::warn!("VMX virtual interrupt delivery failed: {:?}", e);
                    }
                }
                return false;
            }

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

            // Deliver exactly ONE interrupt: LAPIC first, then PIC. The INTA
            // moment itself — priority order, the fabric's counted 8259
            // acknowledge, spurious vectors, the deasserted-pin reconcile —
            // is the shared body; what stays here is delivery into the guest
            // and the VMX folds around it.
            match self.acknowledge_external_interrupt() {
                AcknowledgedInterrupt::Lapic(vector) => {
                    // Bochs event.cc HandleExtInterrupt consults posted-interrupt
                    // processing with the vector the controller just acknowledged:
                    // if it IS the notification vector, it is consumed folding PIR
                    // into the virtual IRR and never reaches the guest as itself.
                    if self.in_vmx_guest && self.vmx_posted_interrupt_processing(vector) {
                        // Consumed; nothing is delivered this boundary.
                    } else {
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
                AcknowledgedInterrupt::Pic(vector) => {
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
                }
                AcknowledgedInterrupt::None => {}
            }
        } else if self.get_hrq() {
            // Assert Hold Acknowledge (HLDA) and perform the DMA transfer.
            // Bochs event.cc makes this the TRAILING arm of the Priority-5
            // chain, so a boundary that delivered an interrupt — or that had
            // interrupts inhibited at the chain head — takes no hold this
            // time round.
            // NOTE: similar code in `handle_wait_for_event` (Bochs event.cc).
            self.device_manager.dma.raise_hlda(&mut *self.memory);
            // Bochs dma.cc raise_HLDA calls bx_pc_system.set_HRQ(0)
            // synchronously at terminal count; apply it here so the
            // async_event clear below observes the dropped line instead
            // of thrashing the trace until the next boundary.
            if let Some(level) = self.device_manager.dma.take_hrq_request() {
                self.pc_system.set_hrq(level);
            }
        }

        // Diagnostic only, and deliberately outside the chain above: an
        // external interrupt that is pending but masked by IF delivers
        // nothing, and in Bochs it does not displace the hold-acknowledge
        // arm either.
        #[cfg(debug_assertions)]
        if !self.is_unmasked_event_pending(
            BxCpuC::<T>::BX_EVENT_PENDING_INTR | BxCpuC::<T>::BX_EVENT_PENDING_LAPIC_INTR,
        ) && (self.pending_event
            & (BxCpuC::<T>::BX_EVENT_PENDING_INTR | BxCpuC::<T>::BX_EVENT_PENDING_LAPIC_INTR))
            != 0
        {
            self.diag_hae_intr_if_blocked += 1;
        }

        // End of handleAsyncEvent: schedule TF->debug_trap for next boundary
        // Bochs event.cc
        if self.eflags.contains(EFlags::TF) {
            self.debug_trap |= BxCpuC::<T>::BX_DEBUG_SINGLE_STEP_BIT;
        }

        // Bochs event.cc: conditionally clear async_event. A guest with GIF
        // clear parks its pending events, so they do not by themselves keep
        // the CPU on the slow path; the monitor trap flag does, whatever GIF
        // says, as does a scheduled debug trap or an asserted HRQ line.
        let has_unmasked_events = (self.pending_event & !self.event_mask) != 0;
        let hrq_active = self.get_hrq();
        if !((self.svm_gif && has_unmasked_events)
            || self.debug_trap != 0
            || self.is_unmasked_event_pending(BxCpuC::<T>::BX_EVENT_VMX_MONITOR_TRAP_FLAG)
            || hrq_active)
        {
            self.async_event = 0;
        }

        // Leaving a HLT/MWAIT sleep becomes visible only here, not in the
        // wake branches of handle_wait_for_event: a VM exit or SMM entry
        // taken on the way out has to report the activity state the CPU was
        // still sleeping in. Paths that leave this function early instead of
        // falling through set it themselves — see `interrupt`, `vmexit` and
        // `svm_vmexit`. Bochs event.cc handleAsyncEvent.
        self.activity_state = CpuActivityState::Active;

        false // Continue execution
    }

    /// One interrupt-acknowledge moment: LAPIC first, then the 8259, in the
    /// priority order Bochs event.cc HandleExtInterrupt takes them.
    ///
    /// The machine's single INTA chain (R5): the interpreter delivers what
    /// this returns into the guest's IDT, and an execution engine injects it
    /// into its partition — both through this body, so the priority order,
    /// the fabric's counted acknowledge and the spurious-vector handling can
    /// never drift between them.
    pub(crate) fn acknowledge_external_interrupt(&mut self) -> AcknowledgedInterrupt {
        // Check LAPIC first (higher priority than PIC in APIC mode)
        if self.lapic.intr {
            // Clear event before acknowledge — acknowledge_int() calls
            // service_local_apic() which may re-signal if more IRQs pending.
            self.clear_event(BxCpuC::<T>::BX_EVENT_PENDING_LAPIC_INTR);
            let vector = self.lapic.acknowledge_int();
            self.sync_lapic_events();
            // Whatever the LAPIC answered is the answer, spurious vectors
            // included: Bochs event.cc interrupt_acknowledge hands the result
            // of acknowledge_int() straight on, and the 8259 sits behind a
            // LAPIC that raised INTR — it does not get a second INTA cycle
            // this boundary.
            #[cfg(debug_assertions)]
            {
                self.diag_hae_intr_delivered += 1;
                self.diag_iac_vectors[vector as usize] += 1;
            }
            self.activity_state = CpuActivityState::Active;
            return AcknowledgedInterrupt::Lapic(vector);
        }

        // Then check PIC (legacy 8259 path) — only if the LAPIC didn't answer
        if self.device_manager.irq.int_pin_asserted() {
            let vector = self.device_manager.irq.acknowledge();
            tracing::trace!("HAE: delivering PIC vector={:#04x} at RIP={:#x} CS={:#06x} mode={:?} IF={}",
            vector, self.rip(), self.sregs[0].selector.value,
            self.cpu_mode, self.eflags.contains(super::eflags::EFlags::IF_));
            // Wake from halt if needed
            self.activity_state = CpuActivityState::Active;
            AcknowledgedInterrupt::Pic(vector)
        } else {
            // The CPU event bit mirrors the PIC INT pin. If no vector is
            // deliverable, reconcile a stale assertion now; otherwise
            // async_event remains set and every instruction exits its
            // trace to rescan an empty PIC indefinitely.
            // Consume deferred PIC edge flags while reconciling the
            // current deasserted INT pin. A later device assertion
            // will set irq_pending again, so it cannot be erased by
            // this acknowledge's stale irq_cleared flag.
            self.device_manager.irq.pic_mut().reconcile_deasserted_intr();
            self.clear_event(BxCpuC::<T>::BX_EVENT_PENDING_INTR);
            if self.pending_event & BxCpuC::<T>::BX_EVENT_PENDING_LAPIC_INTR == 0 {
                self.async_event = super::cpu::BX_ASYNC_EVENT_STOP_TRACE;
            }
            #[cfg(debug_assertions)]
            {
                self.diag_hae_intr_pic_empty += 1;
            }
            AcknowledgedInterrupt::None
        }
    }
}

impl<T: crate::cpu::instrumentation::Instrumentation> BxCpuC<T> {
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

}

/// Startup-IPI delivery runs on the execution context: a SIPI taken in VMX
/// non-root operation exits, and the exit walks the VMEXIT MSR store/load lists
/// in guest memory (Bochs event.cc `deliver_SIPI`).
impl<T: crate::cpu::instrumentation::Instrumentation> ExecCtx<'_, T> {
    /// Bochs `deliver_SIPI`: start a CPU waiting for SIPI at `vector * 0x100`.
    pub(crate) fn deliver_sipi(&mut self, vector: u8) {
        if !matches!(self.activity_state, CpuActivityState::WaitForSipi) {
            tracing::info!(
                "CPU {} started by APIC, but was not halted at that time",
                self.bx_cpuid
            );
            return;
        }

        self.unmask_event(BxCpuC::<T>::BX_EVENT_INIT | BxCpuC::<T>::BX_EVENT_SMI | BxCpuC::<T>::BX_EVENT_NMI);
        // Bochs event.cc deliver_SIPI: SIPI arriving while in VMX non-root
        // operation (guest activity state wait-for-SIPI) causes
        // VMexit(VMX_VMEXIT_SIPI, vector) — the exit unwinds (Bochs longjmp),
        // so the real-mode activation below is skipped;
        // vmexit_load_host_state already sets ACTIVE and clears inhibits.
        // Callers that invoke this from emulator context wire the memory bus
        // first (apply_lapic_cpu_event), so the VMEXIT MSR lists resolve.
        if self.in_vmx_guest {
            self.async_event &= !BxCpuC::<T>::BX_ASYNC_EVENT_SLEEP;
            match self.vmexit_unconditional(VmxVmexitReason::Sipi, vector as u64) {
                Ok(_) | Err(super::error::CpuError::CpuLoopRestart) => {}
                Err(e) => tracing::warn!("VMX SIPI vmexit failed: {:?}", e),
            }
            return;
        }
        self.activity_state = CpuActivityState::Active;
        self.async_event &= !BxCpuC::<T>::BX_ASYNC_EVENT_SLEEP;
        self.set_rip(0);
        self.load_seg_reg_real_mode(BxSegregs::Cs, (vector as u16) << 8);
        tracing::info!(
            "CPU {} started up at {:04X}:{:08X} by APIC",
            self.bx_cpuid,
            (vector as u16) << 8,
            self.eip()
        );
    }
}

impl<T: crate::cpu::instrumentation::Instrumentation> BxCpuC<T> {

    /// Whether this CPU would deliver a Priority-4 debug trap at its next
    /// async-event boundary.
    ///
    /// Bochs event.cc `handleAsyncEvent` orders "traps on the previous
    /// instruction" (TF single-step, data/IO breakpoints, code breakpoints)
    /// at Priority 4, strictly BEFORE external interrupts at Priority 5. The
    /// emulator's PIC-injection path implements only Priority 5, and
    /// `interrupt()` unconditionally clears `debug_trap` (matching Bochs), so
    /// injecting an interrupt while a #DB is pending would destroy it. That
    /// path consults this to defer the interrupt by one boundary.
    #[inline]
    pub(crate) fn debug_trap_pending(&self) -> bool {
        if self.interrupts_inhibited(Self::BX_INHIBIT_DEBUG) {
            return false;
        }
        (self.debug_trap | self.pending_code_breakpoint_trap()) & 0xF000 != 0
    }

    /// Priority-4 code-breakpoint probe for the previous instruction.
    /// Bochs event.cc: `debug_trap |= code_breakpoint_match(get_laddr(
    /// BX_SEG_REG_CS, prev_rip))` — DR0-3 hold LINEAR addresses, so the CS
    /// base must be applied (nonzero in real mode and non-flat protected
    /// segments; zero in 64-bit).
    fn pending_code_breakpoint_trap(&self) -> u32 {
        let laddr = if self.long64_mode() {
            self.get_laddr64(BxSegregs::Cs as usize, self.prev_rip)
        } else {
            u64::from(self.get_laddr32(BxSegregs::Cs as usize, self.prev_rip as u32))
        };
        self.code_breakpoint_match(laddr)
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

    fn make_cpu(cpu_id: u32) -> alloc::boxed::Box<BxCpuC> {
        let mut cpu = BxCpuBuilder::new().build().unwrap();
        cpu.configure_smp(cpu_id, smp_topology());
        cpu
    }

    /// The machine a CPU under test executes against.
    ///
    /// SIPI delivery and async-event servicing live on [`ExecCtx`], so they
    /// need a bus even along the paths that never reach memory — a VMexit taken
    /// out of either one does. The parts outlive each `ctx()` call so state a
    /// test establishes on one call is still there on the next.
    struct TestBus {
        memory: crate::memory::BxMemC,
        devices: crate::iodev::BxDevicesC,
        device_manager: crate::iodev::devices::DeviceManager,
        pc_system: crate::pc_system::BxPcSystemC,
    }

    impl TestBus {
        fn new() -> Self {
            const MIB: usize = 1024 * 1024;
            Self {
                memory: crate::memory::BxMemC::new(
                    crate::memory::BxMemoryStubC::create_and_init(MIB, MIB, 4096).unwrap(),
                    false,
                ),
                devices: crate::iodev::BxDevicesC::new(),
                device_manager: crate::iodev::devices::DeviceManager::new(),
                pc_system: crate::pc_system::BxPcSystemC::new(),
            }
        }

        fn ctx<'a>(&'a mut self, cpu: &'a mut BxCpuC) -> ExecCtx<'a, ()> {
            ExecCtx::new(
                cpu,
                crate::emulator::PcIo::new(
                    &mut self.memory,
                    &mut self.devices,
                    &mut self.device_manager,
                    &mut self.pc_system,
                ),
            )
        }

        /// The same four parts as [`Self::ctx`], as the loan an execution
        /// engine holds — the shape `Emulator` lends to a hardware backend.
        fn io(&mut self) -> crate::emulator::PcIo<'_> {
            crate::emulator::PcIo::new(
                &mut self.memory,
                &mut self.devices,
                &mut self.device_manager,
                &mut self.pc_system,
            )
        }
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
            ap.async_event & BxCpuC::<()>::BX_ASYNC_EVENT_SLEEP,
            0
        );
    }

    #[test]
    fn sipi_starts_only_waiting_application_processor_at_vector_segment() {
        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);

        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);

        assert_eq!(ap.activity_state, CpuActivityState::Active);
        assert_eq!(ap.get_cs_selector(), TEST_SIPI_CS_SELECTOR);
        assert_eq!(ap.get_cs_base(), TEST_SIPI_CS_BASE);
        assert_eq!(ap.rip(), 0);
    }

    #[test]
    fn init_event_software_resets_active_ap_back_to_wait_for_sipi() {
        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);
        ap.set_rax(NONZERO_RAX_SENTINEL);

        ap.deliver_init();
        assert_ne!(
            ap.pending_event & BxCpuC::<()>::BX_EVENT_INIT,
            0
        );
        let exited = bus.ctx(&mut ap).handle_async_event();

        assert!(exited, "AP entering WAIT_FOR_SIPI must exit the cpu loop");
        assert_eq!(ap.rax(), 0);
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);
    }

    #[test]
    fn svm_gif_false_holds_smi_and_init_pending() {
        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);
        ap.set_rax(NONZERO_RAX_SENTINEL);

        // Bochs event.cc handleAsyncEvent: SMI and INIT checks are gated on
        // SVM_GIF; with GIF clear both stay pending and nothing happens.
        ap.svm_gif = false;
        ap.deliver_smi();
        ap.deliver_init();
        let exited = bus.ctx(&mut ap).handle_async_event();

        assert!(!exited);
        assert!(ap.is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_SMI));
        assert!(ap.is_unmasked_event_pending(BxCpuC::<()>::BX_EVENT_INIT));
        assert!(!ap.in_smm, "SMI must not enter SMM while GIF=0");
        assert_eq!(ap.activity_state, CpuActivityState::Active);
        assert_eq!(
            ap.rax(),
            NONZERO_RAX_SENTINEL,
            "INIT must not reset while GIF=0"
        );

        // STGI: with GIF set again, the held INIT is processed. Drop the SMI
        // first so this test does not depend on SMM entry machinery.
        ap.clear_event(BxCpuC::<()>::BX_EVENT_SMI);
        ap.svm_gif = true;
        let exited = bus.ctx(&mut ap).handle_async_event();

        assert!(exited, "AP entering WAIT_FOR_SIPI must exit the cpu loop");
        assert_eq!(ap.rax(), 0);
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);
    }

    #[test]
    fn svm_smi_intercept_takes_vmexit_and_keeps_smi_pending() {
        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);

        ap.in_svm_guest = true;
        let mut vmcb = crate::cpu::svm::VmcbCache::default();
        vmcb.ctrls.intercept_vector[0] |= 1 << crate::cpu::svm::SVM_INTERCEPT0_SMI;
        ap.vmcb = vmcb;

        ap.deliver_smi();
        let exited = bus.ctx(&mut ap).handle_async_event();

        // Bochs event.cc: Svm_Vmexit(SVM_VMEXIT_SMI) fires instead of SMM
        // entry, and the SMI stays pending (held by GIF=0 after the exit).
        assert!(!exited);
        assert!(!ap.in_svm_guest, "SMI intercept must exit SVM guest mode");
        assert!(!ap.svm_gif, "GIF must be clear after SVM VMEXIT");
        assert!(
            ap.pending_event & BxCpuC::<()>::BX_EVENT_SMI != 0,
            "intercepted SMI must stay pending"
        );
        assert!(!ap.in_smm, "SMI intercept must preempt SMM entry");
    }

    #[test]
    fn svm_init_intercept_takes_vmexit_and_keeps_init_pending() {
        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);

        ap.in_svm_guest = true;
        let mut vmcb = crate::cpu::svm::VmcbCache::default();
        vmcb.ctrls.intercept_vector[0] |= 1 << crate::cpu::svm::SVM_INTERCEPT0_INIT;
        ap.vmcb = vmcb;

        ap.deliver_init();
        let exited = bus.ctx(&mut ap).handle_async_event();

        // Bochs event.cc: Svm_Vmexit(SVM_VMEXIT_INIT) fires with INIT still
        // pending; the CPU reset is skipped.
        assert!(!exited);
        assert!(!ap.in_svm_guest, "INIT intercept must exit SVM guest mode");
        assert!(!ap.svm_gif, "GIF must be clear after SVM VMEXIT");
        assert!(
            ap.pending_event & BxCpuC::<()>::BX_EVENT_INIT != 0,
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
            bits & BxCpuC::<()>::BX_DEBUG_TRAP_HIT,
            0,
            "HIT must be set because DR0 is enabled in DR7"
        );

        // A different address does not match.
        assert_eq!(cpu.code_breakpoint_match(0x5678), 0);

        // RF suppresses instruction breakpoints for one instruction.
        cpu.eflags.insert(crate::cpu::eflags::EFlags::RF);
        assert_eq!(cpu.code_breakpoint_match(0x1234), 0);
    }

    /// Bochs event.cc Priority 4 compares DR0-3 against
    /// `get_laddr(BX_SEG_REG_CS, prev_rip)` — the LINEAR address. With a
    /// nonzero CS base (real mode, non-flat protected), matching the raw
    /// EIP misses breakpoints armed on the linear address.
    #[test]
    fn code_breakpoint_check_applies_cs_base() {
        use crate::cpu::crregs::BxDr7;

        let mut cpu = make_cpu(0);
        cpu.reset(ResetReason::Hardware);
        // The reset state already has a nonzero CS base (0xFFFF0000).
        let cs_base = cpu.get_cs_base();
        assert_ne!(cs_base, 0, "reset CS base must be nonzero for this test");

        const EIP: u64 = 0x0123;
        cpu.prev_rip = EIP;
        cpu.dr[0] = cs_base + EIP; // linear breakpoint address
        cpu.dr7 = BxDr7::from_bits_retain(0x1); // L0=1, R/W0=insn, LEN0=1
        cpu.eflags.remove(crate::cpu::eflags::EFlags::RF);

        let bits = cpu.pending_code_breakpoint_trap();
        assert_ne!(
            bits & BxCpuC::<()>::BX_DEBUG_TRAP_HIT,
            0,
            "a DR0 armed on CS.base + EIP must hit (Bochs event.cc \
             code_breakpoint_match(get_laddr(CS, prev_rip)))"
        );

        // The raw EIP itself must NOT match — DR registers hold linear
        // addresses.
        cpu.dr[0] = EIP;
        assert_eq!(cpu.pending_code_breakpoint_trap(), 0);
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
            bits & BxCpuC::<()>::BX_DEBUG_TRAP_HIT,
            0,
            "HIT must NOT be set: DR0 is not enabled in DR7"
        );
    }

    #[test]
    fn smm_entry_masks_smi_and_nmi_until_rsm() {
        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);

        let held = BxCpuC::<()>::BX_EVENT_SMI
            | BxCpuC::<()>::BX_EVENT_NMI
            | BxCpuC::<()>::BX_EVENT_VMX_VIRTUAL_NMI;

        // SMI delivery enters SMM at the next instruction boundary.
        ap.deliver_smi();
        let exited = bus.ctx(&mut ap).handle_async_event();
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
        let exited = bus.ctx(&mut ap).handle_async_event();
        assert!(!exited);
        assert_ne!(
            ap.pending_event & BxCpuC::<()>::BX_EVENT_NMI,
            0,
            "NMI during SMM must stay pending until RSM"
        );
        assert_eq!(
            ap.rip(),
            rip_in_smm,
            "a masked NMI must not be dispatched inside SMM"
        );

        // RSM releases the held events (Bochs smm.cc RSM).
        bus.ctx(&mut ap)
            .rsm(&crate::cpu::decoder::Instruction::default())
            .expect("RSM must succeed outside VMX/SVM guest mode");
        assert!(!ap.in_smm);
        assert_eq!(
            ap.event_mask & held,
            0,
            "RSM must unmask SMI, NMI, and VMX virtual-NMI"
        );
        assert_ne!(
            ap.pending_event & BxCpuC::<()>::BX_EVENT_NMI,
            0,
            "the held NMI is still pending after RSM for the next boundary"
        );
    }

    #[test]
    fn smi_parks_vmx_mode_and_rsm_restores_it() {
        use crate::cpu::crregs::{BxCr0, BxCr4};

        let mut ap = make_cpu(1);
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);

        // Simulate a CPU in VMX non-root operation when the SMI hits.
        ap.cr4.insert(BxCr4::VMXE);
        ap.in_vmx = true;
        ap.in_vmx_guest = true;

        ap.deliver_smi();
        let exited = bus.ctx(&mut ap).handle_async_event();
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
        bus.ctx(&mut ap)
            .rsm(&crate::cpu::decoder::Instruction::default())
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
        let mut bus = TestBus::new();
        ap.reset(ResetReason::Hardware);
        assert_eq!(ap.activity_state, CpuActivityState::WaitForSipi);

        // VMENTRY with guest activity state 3 leaves the CPU in
        // WAIT_FOR_SIPI while in VMX non-root operation.
        ap.in_vmx = true;
        ap.in_vmx_guest = true;

        bus.ctx(&mut ap).deliver_sipi(TEST_SIPI_VECTOR);

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
                & (BxCpuC::<()>::BX_EVENT_SMI | BxCpuC::<()>::BX_EVENT_NMI),
            0,
            "SIPI unmasks SMI/NMI before the VMexit (Bochs deliver_SIPI)"
        );
        assert_ne!(
            ap.event_mask & BxCpuC::<()>::BX_EVENT_INIT,
            0,
            "the VMexit itself re-masks INIT (Bochs vmx.cc: INIT is \
             disabled in VMX root mode)"
        );
    }

    /// IRQ1's line at the fabric.
    const KEYBOARD_LINE: rusty_box_devices::api::IrqLine = rusty_box_devices::api::IrqLine(1);
    /// IRQ1 through the master 8259's power-on offset (pic.rs `new`: 0x08).
    const IRQ1_VECTOR: u8 = 0x09;
    /// The master 8259's spurious vector: its offset + 7 (Bochs pic.cc IAC).
    const SPURIOUS_VECTOR: u8 = 0x0F;

    /// A machine with IRQ1 raised at the fabric and mirrored onto the CPU's
    /// event bit — the pair `sync_io_events` (emulator/io.rs) leaves behind
    /// when a device dispatch latches the PIC line. IF is set so the
    /// interpreter's Priority-5 arm sees the event unmasked.
    fn machine_with_irq1_raised() -> (alloc::boxed::Box<BxCpuC>, TestBus) {
        let mut cpu = make_cpu(0);
        let mut bus = TestBus::new();
        cpu.reset(ResetReason::Hardware);
        // The 8259 powers on with every line masked; unmask them all, as a
        // guest's OCW1 write would.
        bus.device_manager.irq.pic_mut().master.imr = 0x00;
        bus.device_manager.irq.raise(KEYBOARD_LINE);
        cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
        cpu.eflags.insert(EFlags::IF_);
        cpu.handle_interrupt_mask_change();
        (cpu, bus)
    }

    /// The extracted INTA body and the interpreter's delivery agree: raising
    /// a PIC line and letting handle_async_event run delivers the same
    /// vector, through the same counted acknowledge, as popping it directly.
    #[test]
    fn the_popper_and_the_interpreter_acknowledge_identically() {
        // Machine A: the interpreter's own delivery path.
        let (mut cpu, mut bus) = machine_with_irq1_raised();
        assert_eq!(bus.device_manager.irq.acknowledge_count(), 0);

        let exited = bus.ctx(&mut cpu).handle_async_event();

        assert!(!exited);
        assert_eq!(
            bus.device_manager.irq.acknowledge_count(),
            1,
            "the interpreter takes exactly one counted INTA"
        );
        assert_eq!(bus.device_manager.irq.vectors_acknowledged(IRQ1_VECTOR), 1);
        // Delivery really happened: real-mode delivery loads CS:IP from IVT
        // entry 9 (zeroed memory: 0000:0000) and clears IF.
        assert_eq!(cpu.get_cs_selector(), 0);
        assert_eq!(cpu.rip(), 0);
        assert!(!cpu.eflags.contains(EFlags::IF_));

        // Machine B: identical setup, the vector popped directly — the
        // engine's INTA moment.
        let (mut cpu, mut bus) = machine_with_irq1_raised();
        assert_eq!(bus.device_manager.irq.acknowledge_count(), 0);

        let popped = bus.io().pop_deliverable_vector(&mut cpu);

        assert_eq!(popped, Some(IRQ1_VECTOR), "same vector as the interpreter");
        assert_eq!(
            bus.device_manager.irq.acknowledge_count(),
            1,
            "same acknowledge delta as the interpreter"
        );
        assert_eq!(bus.device_manager.irq.vectors_acknowledged(IRQ1_VECTOR), 1);
    }

    /// A line that drops between assertion and acknowledge produces the
    /// 8259's spurious vector through the popper exactly as a real INTA
    /// would — Bochs pic.cc IAC: no unmasked request answers offset + 7.
    #[test]
    fn a_line_lowered_before_the_inta_pops_the_spurious_vector() {
        let (mut cpu, mut bus) = machine_with_irq1_raised();
        // The device drops the line before the acknowledge. The edge-latched
        // INT pin stays asserted — Bochs pic.cc lower_irq clears IRR only.
        bus.device_manager.irq.lower(KEYBOARD_LINE);
        assert!(bus.device_manager.irq.int_pin_asserted());

        let popped = bus.io().pop_deliverable_vector(&mut cpu);

        assert_eq!(popped, Some(SPURIOUS_VECTOR));
        assert_eq!(
            bus.device_manager.irq.acknowledge_count(),
            1,
            "a spurious INTA is still a counted INTA"
        );
        assert_eq!(
            bus.device_manager.irq.vectors_acknowledged(SPURIOUS_VECTOR),
            1
        );
    }

    /// With nothing asserted at the PIC the popper answers None and
    /// reconciles the stale CPU event bit, exactly as the interpreter's
    /// deasserted-pin branch does — and takes no INTA doing it.
    #[test]
    fn an_empty_pic_pops_none_and_reconciles_the_stale_event_bit() {
        let mut cpu = make_cpu(0);
        let mut bus = TestBus::new();
        cpu.reset(ResetReason::Hardware);
        // A stale assertion: the event bit set with the INT pin deasserted.
        cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);

        let popped = bus.io().pop_deliverable_vector(&mut cpu);

        assert_eq!(popped, None);
        assert_eq!(
            bus.device_manager.irq.acknowledge_count(),
            0,
            "no INTA on an empty PIC"
        );
        assert_eq!(
            cpu.pending_event & BxCpuC::<()>::BX_EVENT_PENDING_INTR,
            0,
            "the stale event bit is reconciled"
        );
        assert_eq!(
            cpu.async_event,
            BxCpuC::<()>::BX_ASYNC_EVENT_STOP_TRACE,
            "the boundary ends the trace instead of rescanning an empty PIC"
        );
    }

    /// A LAPIC vector in priority class 0x40 — above the power-on PPR of 0,
    /// so the LAPIC will deliver it, and distinct from every 8259 vector.
    const LAPIC_VECTOR: u8 = 0x41;
    /// xAPIC spurious-vector register offset (Bochs apic.h BX_LAPIC_SPURIOUS_VECTOR).
    const LAPIC_SVR_OFFSET: u64 = 0xF0;
    /// The guest-visible IRR/ISR word holding [`LAPIC_VECTOR`]'s bit:
    /// word 0x41 / 32 = 2 of the register file (Bochs apic.cc read_aligned).
    const LAPIC_VECTOR_ISR_OFFSET: u64 = 0x120;
    const LAPIC_VECTOR_IRR_OFFSET: u64 = 0x220;
    /// Bit 0x41 % 32 = 1 inside that word.
    const LAPIC_VECTOR_REG_BIT: u32 = 1 << 1;

    /// With BOTH controllers armed, the LAPIC answers the boundary and the
    /// 8259 is left untouched — Bochs event.cc interrupt_acknowledge
    /// consults the local APIC before DEV_pic_iac. The 8259's request must
    /// survive the LAPIC's turn: the next pop answers with the PIC's vector.
    #[test]
    fn the_lapic_outranks_the_pic_and_the_pic_request_survives() {
        use crate::cpu::apic::{ApicDeliveryMode, APIC_EDGE_TRIGGERED};

        let (mut cpu, mut bus) = machine_with_irq1_raised();
        // The guest software-enables its LAPIC before use — an SVR write
        // with bit 8 set, keeping the power-on spurious vector 0xFF.
        cpu.lapic.write_aligned(LAPIC_SVR_OFFSET, 0x1FF, 0);
        // A fixed interrupt arrives over the APIC bus and lands in the IRR.
        assert!(
            cpu.lapic.deliver(
                LAPIC_VECTOR,
                ApicDeliveryMode::Fixed as u8,
                APIC_EDGE_TRIGGERED
            ),
            "fixed delivery is accepted"
        );
        assert!(cpu.lapic.intr, "service_local_apic raised INTR");
        cpu.sync_lapic_events();
        assert_ne!(
            cpu.pending_event & BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR,
            0,
            "the sync point mirrored INTR onto the CPU event bit"
        );

        let first = bus.io().pop_deliverable_vector(&mut cpu);

        assert_eq!(
            first,
            Some(LAPIC_VECTOR),
            "the LAPIC's vector wins the boundary"
        );
        assert_eq!(
            bus.device_manager.irq.acknowledge_count(),
            0,
            "the 8259 saw no INTA while the LAPIC answered"
        );
        // The vector really moved IRR -> ISR: the guest-visible ISR word
        // reads it back, and the LAPIC's INTR line dropped.
        assert_eq!(
            cpu.lapic.read_aligned(LAPIC_VECTOR_ISR_OFFSET, 0) & LAPIC_VECTOR_REG_BIT,
            LAPIC_VECTOR_REG_BIT,
            "the acknowledged vector is in service"
        );
        assert!(!cpu.lapic.intr, "the LAPIC consumed its request");

        // The PIC's request was preserved, not dropped: the next boundary
        // answers with it, through the fabric's counted INTA.
        let second = bus.io().pop_deliverable_vector(&mut cpu);

        assert_eq!(
            second,
            Some(IRQ1_VECTOR),
            "the 8259's request survived the LAPIC's turn"
        );
        assert_eq!(bus.device_manager.irq.acknowledge_count(), 1);
        assert_eq!(bus.device_manager.irq.vectors_acknowledged(IRQ1_VECTOR), 1);
    }

    /// Vector 0 is a vector. With the SVR's vector bits programmed to 0 (the
    /// xAPIC model keeps all eight writable — Bochs apic.cc
    /// write_spurious_interrupt_register) a nothing-deliverable acknowledge
    /// answers 0 (Bochs apic.cc acknowledge_int returns spurious_vector), and
    /// Bochs event.cc interrupt_acknowledge hands that answer on unchanged.
    /// The 8259 sat behind a LAPIC that raised INTR, so it does not get an
    /// INTA cycle this boundary — its request stays latched for the next one.
    #[test]
    fn a_zero_vector_from_the_lapic_is_delivered_and_the_pic_waits() {
        use crate::cpu::apic::{ApicDeliveryMode, APIC_EDGE_TRIGGERED};

        let (mut cpu, mut bus) = machine_with_irq1_raised();
        // The guest software-enables the LAPIC with spurious vector 0.
        cpu.lapic.write_aligned(LAPIC_SVR_OFFSET, 0x100, 0);
        // A fixed interrupt raises INTR...
        assert!(cpu.lapic.deliver(
            LAPIC_VECTOR,
            ApicDeliveryMode::Fixed as u8,
            APIC_EDGE_TRIGGERED
        ));
        assert!(cpu.lapic.intr);
        cpu.sync_lapic_events();
        // ...and the guest raises TPR to the vector's class before the
        // INTA — the race that makes the acknowledge answer the spurious
        // vector (Bochs apic.cc acknowledge_int: vector & 0xf0 <= get_ppr()).
        cpu.lapic.set_tpr(LAPIC_VECTOR);
        assert!(
            cpu.lapic.intr,
            "raising TPR does not lower an already-raised INTR"
        );

        let popped = bus.io().pop_deliverable_vector(&mut cpu);

        assert_eq!(
            popped,
            Some(0),
            "the LAPIC's spurious vector is the answer, zero included"
        );
        assert_eq!(
            bus.device_manager.irq.acknowledge_count(),
            0,
            "the 8259 was not acknowledged behind a LAPIC that answered"
        );
        assert!(
            bus.device_manager.irq.int_pin_asserted(),
            "the 8259 still holds its request for the next boundary"
        );
        assert!(!cpu.lapic.intr, "the spurious acknowledge lowered INTR");
        // The blocked request stays latched for when TPR drops: the
        // guest-visible IRR word still holds the vector's bit.
        assert_eq!(
            cpu.lapic.read_aligned(LAPIC_VECTOR_IRR_OFFSET, 0) & LAPIC_VECTOR_REG_BIT,
            LAPIC_VECTOR_REG_BIT,
            "the TPR-blocked request is not dropped"
        );
    }

    /// A halted processor is woken by every event Bochs wakes it for.
    ///
    /// `handleWaitForEvent` (event.cc) ends the wait for NMI/SMI/INIT *and*
    /// for the events only a VMEXIT can answer — the virtual-APIC updates,
    /// the monitor trap flag and the virtual NMI. `handleAsyncEvent` calls
    /// it first and returns immediately when it says "still halted", so an
    /// event missing from this condition is one the processor can never
    /// reach: the VMEXIT below it never runs.
    #[test]
    fn a_halted_processor_wakes_for_the_events_only_a_vmexit_can_answer() {
        for event in [
            BxCpuC::<()>::BX_EVENT_VMX_MONITOR_TRAP_FLAG,
            BxCpuC::<()>::BX_EVENT_VMX_VIRTUAL_NMI,
            BxCpuC::<()>::BX_EVENT_VMX_VTPR_UPDATE,
            BxCpuC::<()>::BX_EVENT_VMX_VEOI_UPDATE,
            BxCpuC::<()>::BX_EVENT_VMX_VIRTUAL_APIC_WRITE,
        ] {
            let mut cpu = make_cpu(0);
            let mut bus = TestBus::new();
            cpu.reset(ResetReason::Hardware);
            cpu.activity_state = CpuActivityState::Hlt;
            cpu.unmask_event(event);
            cpu.signal_event(event);

            let mut ctx = bus.ctx(&mut cpu);

            assert!(
                !ctx.handle_wait_for_event(),
                "event {event:#x} must end the wait so its VMEXIT can be taken"
            );
        }
    }

    /// The expired VMX preemption timer ends the wait without the side
    /// effects an interrupt wake has.
    ///
    /// Bochs event.cc gives it a branch of its own, after the interrupt
    /// condition: it breaks out of the wait loop but does not reset the
    /// MONITOR and does not clear the inhibit mask, because nothing was
    /// delivered — the processor is only being let go so the VMEXIT can run.
    #[test]
    fn an_expired_preemption_timer_ends_the_wait_without_clearing_inhibits() {
        const INHIBITED: u32 = 1;

        let mut cpu = make_cpu(0);
        let mut bus = TestBus::new();
        cpu.reset(ResetReason::Hardware);
        cpu.activity_state = CpuActivityState::Mwait;
        cpu.monitor.arm(0x1000, crate::cpu::cpu::BX_MONITOR_ARMED_BY_MONITOR);
        cpu.inhibit_mask = INHIBITED;
        cpu.unmask_event(BxCpuC::<()>::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);
        cpu.signal_event(BxCpuC::<()>::BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED);

        let mut ctx = bus.ctx(&mut cpu);
        assert!(
            !ctx.handle_wait_for_event(),
            "an expired preemption timer ends the wait"
        );

        assert!(
            cpu.monitor.armed(),
            "the preemption timer delivered nothing, so the MONITOR stays armed"
        );
        assert_eq!(
            cpu.inhibit_mask, INHIBITED,
            "the preemption timer does not clear the inhibits an interrupt wake clears"
        );
    }

    /// A user interrupt wakes a halted processor on the same terms as any
    /// other interrupt: Bochs event.cc puts `BX_EVENT_PENDING_UINTR` in the
    /// group gated on EFLAGS.IF (or the MWAIT_IF state MWAIT's ECX[0] asks
    /// for), alongside the 8259's and the LAPIC's.
    #[test]
    fn a_user_interrupt_wakes_a_halted_processor_only_when_interrupts_are_enabled() {
        let mut cpu = make_cpu(0);
        let mut bus = TestBus::new();
        cpu.reset(ResetReason::Hardware);
        cpu.activity_state = CpuActivityState::Hlt;
        cpu.eflags.remove(super::super::eflags::EFlags::IF_);
        cpu.unmask_event(BxCpuC::<()>::BX_EVENT_PENDING_UINTR);
        cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_UINTR);

        assert!(
            bus.ctx(&mut cpu).handle_wait_for_event(),
            "with interrupts disabled the wait continues"
        );

        cpu.eflags.insert(super::super::eflags::EFlags::IF_);

        assert!(
            !bus.ctx(&mut cpu).handle_wait_for_event(),
            "with interrupts enabled the user interrupt ends the wait"
        );
    }
}
