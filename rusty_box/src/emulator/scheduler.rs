use crate::{
    cpu::{
        apic::{LocalApicCpuEvent, LocalApicTimerActivation, PendingIpi},
        cpu::CpuActivityState,
        instrumentation::Instrumentation,
        BxCpuC, CpuError, Result as CpuResult,
    },
    Result,
};


use super::{CpuMask, Emulator, Progress, SliceEngine, SliceRequest, BOCHS_APIC_BUS_ID_MASK};

impl<'a, T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    /// Invalidate every host pointer and decoded trace before memory backing
    /// can be replaced or restored.
    pub(crate) fn invalidate_all_cpu_host_mappings(&mut self) {
        let smc_seq = self.memory.smc_seq_next();
        for cpu_index in 0..self.cpu_count() {
            let cpu = self.cpu_mut_at(cpu_index);
            cpu.invalidate_host_memory_mappings();
            // A full icache flush consumes every memory-side SMC event,
            // including a snapshot restore that restarted the sequence.
            cpu.smc_seq_seen = smc_seq;
        }
    }

    pub(super) fn cpu_runnable_for_batch(&self, index: usize) -> bool {
        let cpu = self.cpu_ref(index);
        match cpu.activity_state {
            // Bochs event.cc handleWaitForEvent: only WAIT_FOR_SIPI returns to
            // the caller without wake checks. SHUTDOWN shares the HLT wake set
            // below (unmasked NMI/SMI/INIT, or INTR/LAPIC-INTR with IF).
            CpuActivityState::WaitForSipi => false,
            CpuActivityState::Active => true,
            CpuActivityState::MwaitIf => {
                cpu.is_unmasked_event_pending(u32::MAX)
                    || cpu.lapic.intr
                    || (cpu.pending_event
                        & (BxCpuC::<()>::BX_EVENT_PENDING_INTR
                            | BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR))
                        != 0
            }
            _ => {

                cpu.is_unmasked_event_pending(u32::MAX)
                    || (cpu.lapic.intr && cpu.interrupts_enabled())
            }
        }
    }

    /// Refresh authoritative membership after an observed CPU/LAPIC transition.
    ///
    /// Every mutation path that can change runnability or deferred LAPIC work
    /// must call this before returning to the scheduler.
    pub(super) fn refresh_cpu_masks(&mut self, index: usize) {
        let (runnable, lapic_work) = {
            let cpu = self.cpu_ref(index);
            (
                self.cpu_runnable_for_batch(index),
                cpu.lapic.has_scheduler_work() || cpu.lapic.timer_fired,
            )
        };
        self.runnable_mask.assign(index, runnable);
        self.lapic_work_mask.assign(index, lapic_work);
    }

    /// Rebuild every derived mask from architectural CPU state.
    pub(crate) fn rebuild_cpu_masks_from_scan(&mut self) {
        self.runnable_mask = CpuMask::default();
        self.lapic_work_mask = CpuMask::default();
        for index in 0..self.cpu_count() {
            self.refresh_cpu_masks(index);
        }
    }

    #[cfg(test)]
    fn scanned_cpu_masks(&self) -> (CpuMask, CpuMask) {
        let mut runnable = CpuMask::default();
        let mut lapic_work = CpuMask::default();
        for index in 0..self.cpu_count() {
            runnable.assign(index, self.cpu_runnable_for_batch(index));
            let lapic = &self.cpu_ref(index).lapic;
            lapic_work.assign(index, lapic.has_scheduler_work() || lapic.timer_fired);
        }
        (runnable, lapic_work)
    }

    #[cfg(test)]
    pub(super) fn assert_cpu_masks_match_scan(&self) {
        let (runnable, lapic_work) = self.scanned_cpu_masks();
        assert_eq!(self.runnable_mask, runnable, "runnable CPU mask diverged");
        assert_eq!(
            self.lapic_work_mask, lapic_work,
            "LAPIC work CPU mask diverged"
        );
        assert_eq!(
            self.can_fast_forward_bsp_hlt(),
            self.can_fast_forward_bsp_hlt_scan(),
            "HLT fast-forward predicate diverged from the per-AP scan"
        );
    }

    /// Per-AP HLT fast-forward eligibility from the maintained runnable mask.
    ///
    /// Equivalence with the authoritative per-AP scan
    /// (`can_fast_forward_bsp_hlt_scan`): `cpu_runnable_for_batch` is false
    /// exactly for WAIT_FOR_SIPI and for the SHUTDOWN/HLT/MWAIT family with
    /// no pending wake event — precisely the scan's eligibility set — and
    /// `Active` is always runnable. The test-mode oracle asserts this
    /// equivalence at every batch/boundary.
    #[inline]
    pub(super) fn ap_fast_forward_allowed(runnable_mask: CpuMask, cpu_count: usize) -> bool {
        runnable_mask.next_set(1, cpu_count).is_none()
    }

    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    pub(super) fn can_fast_forward_bsp_hlt(&self) -> bool {
        // CPUs that have not received SIPI, or are shut down / halted with no
        // runnable event, do not require round-robin slices. Keep CPU0's
        // single-CPU HLT/MWAIT pacing path available in those states; otherwise
        // idle APs make the emulator bounce through trace-sized batches.
        // SHUTDOWN is runnability-gated like HLT (Bochs event.cc
        // handleWaitForEvent wakes it on unmasked NMI/SMI/INIT), so a shutdown
        // AP holding a pending wake event must not be fast-forwarded past.
        //
        // Bochs itself never fast-forwards in SMP mode — it grinds empty
        // rounds crediting each idle CPU one quantum. Jumping straight to the
        // next pc_system deadline is observationally identical (timers fire
        // at their exact deadlines inside tickn either way, and a fully idle
        // machine has no other event source), so this host-side optimization
        // does not diverge from Bochs guest-visible behavior.
        Self::ap_fast_forward_allowed(self.runnable_mask, self.cpu_count())
    }

    /// The authoritative per-AP scan the mask predicate must match; kept as
    /// the test oracle only.
    #[cfg(test)]
    fn can_fast_forward_bsp_hlt_scan(&self) -> bool {
        (1..self.cpu_count()).all(|cpu_index| {
            matches!(
                self.cpu_ref(cpu_index).activity_state,
                CpuActivityState::WaitForSipi
            ) || (matches!(
                self.cpu_ref(cpu_index).activity_state,
                CpuActivityState::Shutdown
                    | CpuActivityState::Hlt
                    | CpuActivityState::Mwait
                    | CpuActivityState::MwaitIf
            ) && !self.cpu_runnable_for_batch(cpu_index))
        })
    }

    /// Run a CPU batch.
    ///
    /// The CPU, memory and both buses reach the loop as borrows through
    /// `ExecCtx`; what raw wiring remains is device-side, installed and
    /// cleared entirely within this call, so `&mut self` is the whole
    /// contract and there is nothing left for a caller to uphold.
    pub fn run_cpu_batch(&mut self, batch_size: u64) -> CpuResult<Progress> {
        self.run_cpu_batch_with_strict_limit(batch_size, false)
    }

    pub(super) fn run_cpu_batch_with_strict_limit(
        &mut self,
        batch_size: u64,
        strict_limit: bool,
    ) -> CpuResult<Progress> {
        if self.snapshot_restore_failed {
            return Err(CpuError::CpuNotInitialized);
        }
        self.batch_advanced_pc_system = false;
        // A reset applied at batch entry needs no special handling: the batch
        // below simply starts executing at the reset vector.
        self.service_scheduler_boundary(0)?;

        let cpu_count = self.cpu_count();
        let smp = cpu_count > 1;
        let mut total_elapsed_ticks = 0u64;
        let mut total_up_executed = 0u64;
        let mut result: CpuResult<()> = Ok(());
        let initial_deadline_ticks =
            u64::from(self.pc_system.get_num_cpu_ticks_left_next_event());
        let strict_up_deadline =
            !smp && (strict_limit || initial_deadline_ticks <= batch_size);
        let batch_size = if smp {
            batch_size
        } else {
            batch_size.min(initial_deadline_ticks).max(1)
        };

        while total_elapsed_ticks < batch_size {
            let runnable_count = self.runnable_mask.count(cpu_count);
            if runnable_count == 0 {
                break;
            }

            let remaining = batch_size.saturating_sub(total_elapsed_ticks);
            let round_deadline_ticks =
                u64::from(self.pc_system.get_num_cpu_ticks_left_next_event());
            let unconstrained_per_cpu_batch = self.smp_quantum_ticks().min(remaining.max(1));
            let strict_smp_deadline =
                smp && (strict_limit || round_deadline_ticks <= unconstrained_per_cpu_batch);
            let per_cpu_batch = if smp {
                unconstrained_per_cpu_batch
                    .min(round_deadline_ticks)
                    .max(1)
            } else {
                (remaining / runnable_count as u64).max(1)
            };
            let mut round_ticks = 0u64;
            let mut boundary_reached = false;
            let mut reset_in_round = false;
            let idle_credit = self.smp_quantum_ticks();
            let mut cpu_cursor = 0usize;

            loop {
                let Some(cpu_index) = self.runnable_mask.next_set(cpu_cursor, cpu_count) else {
                    if smp {
                        round_ticks = round_ticks.saturating_add(
                            (cpu_count - cpu_cursor) as u64 * idle_credit,
                        );
                    }
                    break;
                };
                if smp {
                    round_ticks = round_ticks.saturating_add(
                        (cpu_index - cpu_cursor) as u64 * idle_credit,
                    );
                }
                cpu_cursor = cpu_index + 1;
                debug_assert!(self.cpu_runnable_for_batch(cpu_index));

                if smp {
                    // Stamp this CPU's LAPIC time epoch with the round clock.
                    // Bochs main.cc SMP loop: `bx_pc_system.ticksTotal` grows
                    // by BX_TICKN once per full round, so `time_ticks()` —
                    // which apic.cc get_current_timer_count reads — is frozen
                    // within a trace but ADVANCES between rounds. Without this
                    // stamp, a CPU whose LAPIC has no queued scheduler work
                    // keeps a stale epoch and its TMCCT appears dead until the
                    // timer fires — guest APIC-timer calibration (TMICT armed
                    // huge, TMCCT polled over a PIT window) then never sees
                    // the count move and hangs the AP bring-up.
                    let round_epoch = self.pc_system.time_ticks();
                    let cpu = self.cpu_mut_at(cpu_index);
                    cpu.mark_tick_sync();
                    cpu.lapic.current_ticks = round_epoch;
                    cpu.lapic.ticks_at_sync = round_epoch;
                    cpu.lapic.cpu_ticks_at_sync = cpu.cpu_ticks();
                }

                let ticks_before = self.cpu_ref(cpu_index).cpu_ticks();
                // One borrow of the machine hands the engine this processor
                // and the parts it may touch. Scoped to the call, so the
                // bookkeeping below can use `self` again.
                //
                // An SMP round returns after one trace so the next processor
                // can have the machine, and credits a full round with one
                // tick; a uniprocessor has neither to do.
                let slice_result = self.run_slice(
                    cpu_index,
                    SliceRequest {
                        instructions: per_cpu_batch,
                        strict: if smp { strict_smp_deadline } else { strict_up_deadline },
                        tick_denominator: if smp { cpu_count as u64 } else { 1 },
                        yield_after_one_trace: smp,
                    },
                );

                let boundary_requested =
                    self.cpu_mut_at(cpu_index).take_scheduler_boundary_request();
                self.refresh_cpu_masks(cpu_index);

                match slice_result {
                    Ok(executed) => {
                        if !smp {
                            total_up_executed = total_up_executed.saturating_add(executed);
                        }
                        let elapsed = if smp {
                            let delta = self.cpu_ref(cpu_index).tick_delta_since_sync();
                            if delta == 0 {
                                self.smp_quantum_ticks()
                            } else {
                                delta
                            }
                        } else {
                            self.cpu_ref(cpu_index)
                                .cpu_ticks()
                                .saturating_sub(ticks_before)
                        };
                        round_ticks = if smp {
                            round_ticks.saturating_add(elapsed)
                        } else {
                            round_ticks.max(elapsed)
                        };

                        // SMP must expose queued work before a sibling runs.
                        // UP services a distinct boundary immediately; elapsed
                        // virtual time is committed below.
                        //
                        // A slice that queued nothing needs no boundary: the
                        // exact predicate covers every source the boundary
                        // drains, so skipping is a pure no-op elision. This
                        // keeps the SMP round loop near Bochs main.cc cost,
                        // where slices run no device servicing at all.
                        if (smp || boundary_requested)
                            && (boundary_requested || self.scheduler_boundary_work_pending())
                        {
                            if self.service_scheduler_boundary(0)? {
                                // Reset applied mid-round: discard all
                                // pre-reset round time (including the SMP
                                // division remainder) so no pre-reset tick
                                // reaches the fresh machine, and end the
                                // batch at the reset vector.
                                round_ticks = 0;
                                self.smp_tick_remainder = 0;
                                reset_in_round = true;
                                boundary_reached = true;
                                break;
                            }
                        }
                        if boundary_requested {
                            boundary_reached = true;
                            break;
                        }
                    }
                    Err(err) => {
                        result = Err(err);
                        break;
                    }
                }
            }

            #[cfg(test)]
            self.assert_cpu_masks_match_scan();

            if result.is_err() {
                break;
            }

            if reset_in_round {
                // Pre-reset elapsed time was discarded; the next instruction
                // executed is at the reset vector.
                break;
            }

            let elapsed_ticks = if smp {
                let total_ticks = self.smp_tick_remainder.saturating_add(round_ticks);
                let elapsed = total_ticks / cpu_count as u64;
                self.smp_tick_remainder = total_ticks % cpu_count as u64;
                elapsed
            } else {
                round_ticks
            };
            if elapsed_ticks == 0 {
                if round_ticks == 0 {
                    break;
                }
                continue;
            }

            if self.service_scheduler_boundary(elapsed_ticks)? {
                // Reset at the commit boundary itself: elapsed_ticks was
                // discarded by the boundary and must not be reported as
                // committed batch time.
                self.smp_tick_remainder = 0;
                break;
            }
            total_elapsed_ticks = total_elapsed_ticks.saturating_add(elapsed_ticks);
            self.batch_advanced_pc_system = true;
            if boundary_reached {
                break;
            }
            if !smp {
                break;
            }
        }

        #[cfg(test)]
        self.assert_cpu_masks_match_scan();
        self.drain_hook_stop_requests();
        result.map(|_| {
            // Bochs main.cc: an SMP round credits every processor a quantum and
            // advances machine time by the round's average, so what this batch
            // advanced is a span of time and not any processor's instruction
            // count. A uniprocessor's own count IS the machine's, so it says so.
            if smp {
                Progress::Ticks(total_elapsed_ticks)
            } else {
                Progress::Instructions(total_up_executed)
            }
        })
    }

    /// Turn a stop honoured inside the CPU loop into the machine's own stop.
    ///
    /// The one place a hook's request crosses from a processor to the machine
    /// (doctrine R5), and it deliberately raises the same flag a host
    /// `StopHandle` raises rather than inventing a parallel signal — so every
    /// run loop that already stops for a host request stops for a hook too,
    /// and none of them needs to know hooks exist.
    ///
    /// The flag is left for the caller to observe and clear; only the per-CPU
    /// report is consumed here, so one honoured request raises the machine's
    /// flag exactly once.
    fn drain_hook_stop_requests(&mut self) {
        let mut honored = false;
        for index in 0..self.cpu_count() {
            honored |= core::mem::take(&mut self.cpu_mut_at(index).instrumentation.stop_honored);
        }
        if honored {
            self.stop_flag
                .store(true, core::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Bochs `cpu: quantum=N` (BXPN_SMP_QUANTUM), clamped to config.h
    /// BX_SMP_QUANTUM_MIN..=BX_SMP_QUANTUM_MAX.
    #[inline]
    pub(super) fn smp_quantum_ticks(&self) -> u64 {
        (self.config.smp_quantum as u64).clamp(1, 32)
    }


    /// Apply queued SMC invalidations to every cpu, then drop the queue.
    ///
    /// Bochs icache.cc `handleSMC`: on a write hitting stamped lines, every
    /// processor gets `async_event |= BX_ASYNC_EVENT_STOP_TRACE` and an
    /// icache flush, synchronously. Here the writing cpu (or device) queued
    /// the event; this drain runs at slice/round/batch boundaries — before
    /// any sibling cpu can execute — so guest-visible behavior is identical.
    /// Per-cpu `smc_seq_seen` watermarks make repeat calls O(1) when nothing
    /// is pending.
    fn drain_pending_smc(&mut self) {
        if !self.memory.smc_has_pending() {
            // Empty queue ⇒ every cpu already caught up (the queue is only
            // cleared after a full catch-up) — one load per slice.
            return;
        }
        let newest = self.memory.smc_seq_next();
        for cpu_index in 0..self.cpu_count() {
            if self.cpu_ref(cpu_index).smc_seq_seen < newest {
                // The CPU and memory are distinct fields, which an execution
                // context is exactly the borrow that expresses.
                let mut ctx = self.exec_ctx(cpu_index);
                let (cpu, mem, _devices, _pc_system) = ctx.slice_parts();
                cpu.smc_apply_pending(mem, true);
            }
        }
        self.memory.smc_clear_pending();
    }

    /// Inject an external interrupt.
    ///
    /// Delivery reads the IVT/IDT and pushes a stack frame, so it runs on an
    /// execution context — assembling one is what gives it memory.
    ///
    /// Used by `run_interactive` / `step_batch` for manual interrupt delivery
    /// between CPU batches. Also available for no-alloc callers doing their
    /// own batch loops (e.g. UEFI example).
    pub fn inject_interrupt(&mut self, vector: u8) -> CpuResult<()> {
        let result = self.exec_ctx(0).inject_external_interrupt(vector);
        self.refresh_cpu_masks(0);
        result
    }
}

impl<'a, T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    /// Check for pending reset requests (keyboard 0xFE, port 92h, PCI CF9).
    /// If a reset is pending, clears the request flags and performs that reset type.
    /// Returns true if a reset was performed.
    pub fn check_and_handle_resets(&mut self) -> Result<bool> {
        let Some(reset_type) = self.device_manager.take_reset_request() else {
            return Ok(false);
        };
        self.reset(reset_type)?;
        Ok(true)
    }


    /// Check if an interrupt is pending (PIC or LAPIC)
    pub fn has_interrupt(&self) -> bool {
        // Legacy PIC path
        if self.device_manager.has_interrupt() {
            return true;
        }
        // APIC path: check LAPIC for pending interrupts
        if self.cpu_ref(0).lapic_has_intr() {
            return true;
        }
        false
    }

    /// Acknowledge interrupt and get vector
    pub fn iac(&mut self) -> u8 {
        self.device_manager.iac()
    }

    /// Advance a fully halted machine directly to its earliest exact timer
    /// deadline. Host input is pumped before each halted step.
    ///
    /// Bochs event.cc `handleWaitForEvent` idles a single CPU by spinning on
    /// `BX_TICKN(10)` and returns to `cpu_loop` only for `BX_SMP_PROCESSORS > 1`;
    /// this returns on every configuration, and the scheduler advances time in
    /// its place. Timers fire at their exact deadlines inside `tickn` either
    /// way, and a fully idle machine has no other event source, so the guest
    /// cannot observe which loop moved the clock. Declared divergence D1 in
    /// `docs/bochs-parity-divergences.md`, which records the measurement that
    /// rules out adopting the tick granularity here.
    #[inline]
    pub(super) fn hlt_wait_step_ticks(&self) -> u32 {
        self.pc_system.get_num_cpu_ticks_left_next_event().max(1)
    }

    pub(super) fn drain_lapic_bus(&mut self) {
        let cpu_count = self.cpu_count();
        let mut cursor = 0usize;
        while let Some(src) = self.lapic_work_mask.next_set(cursor, cpu_count) {
            cursor = src + 1;
            self.drain_lapic_bus_from(cpu_count, src);
        }
    }

    /// Drain queued ICR IPIs from one CPU selected by `lapic_work_mask`.
    fn drain_lapic_bus_from(&mut self, cpu_count: usize, src: usize) {
        while let Some(ipi) = { self.cpu_mut_at(src).lapic.take_pending_ipi() } {
            self.deliver_pending_ipi(cpu_count, src, ipi);
        }
        self.refresh_cpu_masks(src);
    }


    fn deliver_lapic_bus_interrupt(
        &mut self,
        target: usize,
        vector: u8,
        delivery_mode: u8,
        trigger_mode: u8,
    ) {
        let (cpu_event, signal_lapic_intr) = {
            let cpu = self.cpu_mut_at(target);
            cpu.lapic.deliver(vector, delivery_mode, trigger_mode);
            let cpu_event = cpu.lapic.take_pending_cpu_event();
            let signal_lapic_intr = cpu.lapic.intr_pending;
            if signal_lapic_intr {
                cpu.lapic.intr_pending = false;
            }
            (cpu_event, signal_lapic_intr)
        };
        self.apply_lapic_cpu_event(target, cpu_event);
        if signal_lapic_intr {
            self.cpu_mut_at(target)
                .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
        }
        self.refresh_cpu_masks(target);
    }

    fn deliver_pending_ipi(&mut self, cpu_count: usize, src: usize, ipi: PendingIpi) {
        let mut accepted = ipi.accepted;
        let vector = (ipi.lo_cmd & 0xFF) as u8;
        let delivery_mode = ((ipi.lo_cmd >> 8) & 7) as u8;
        let trigger_mode = ((ipi.lo_cmd >> 15) & 1) as u8;

        if delivery_mode == 1 {
            if !self.cpu_ref(0).lapic.is_xapic() {
                let mut focus_target = None;
                for target in 0..cpu_count {
                    if self.cpu_ref(target).lapic.is_focus(vector) {
                        focus_target = Some(target);
                        break;
                    }
                }
                if let Some(target) = focus_target {
                    self.deliver_lapic_bus_interrupt(target, vector, delivery_mode, trigger_mode);
                    accepted = true;
                }
            }

            if !accepted {
                let mut selected = None;
                for target in 0..cpu_count {
                    if !self.ipi_targets_cpu(src, ipi, target) {
                        continue;
                    }
                    let priority = {
                        let lapic = &self.cpu_ref(target).lapic;
                        if lapic.is_xapic() {
                            lapic.get_tpr()
                        } else {
                            lapic.get_apr()
                        }
                    };
                    if selected
                        .map(|(_, best_priority)| priority < best_priority)
                        .unwrap_or(true)
                    {
                        selected = Some((target, priority));
                    }
                }
                if let Some((target, _)) = selected {
                    self.deliver_lapic_bus_interrupt(target, vector, delivery_mode, trigger_mode);
                    accepted = true;
                }
            }

            if !accepted {
                self.cpu_mut_at(src).lapic.record_tx_accept_error();
            }
            return;
        }

        for target in 0..cpu_count {
            if !self.ipi_targets_cpu(src, ipi, target) {
                continue;
            }
            self.deliver_lapic_bus_interrupt(target, vector, delivery_mode, trigger_mode);
            accepted = true;
        }
        if !accepted {
            self.cpu_mut_at(src).lapic.record_tx_accept_error();
        }
    }

    fn ipi_targets_cpu(&self, src: usize, ipi: PendingIpi, target: usize) -> bool {
        if ipi.exclude_source && target == src {
            return false;
        }

        match ipi.shorthand {
            0 => {
                let target_lapic = &self.cpu_ref(target).lapic;
                let logical_dest = (ipi.lo_cmd >> 11) & 1 != 0;
                if logical_dest {
                    target_lapic.matches_logical_dest(ipi.dest)
                } else {
                    ipi.dest == target_lapic.get_id()
                        || (ipi.dest & BOCHS_APIC_BUS_ID_MASK) == BOCHS_APIC_BUS_ID_MASK
                }
            }
            2 => true,
            3 => target != src,
            _ => false,
        }
    }

    fn ioapic_targets_cpu(
        &self,
        delivery: crate::iodev::ioapic::PendingIoApicDelivery,
        target: usize,
    ) -> bool {
        let target_lapic = &self.cpu_ref(target).lapic;
        if delivery.dest_mode != 0 {
            target_lapic.matches_logical_dest(delivery.dest)
        } else {
            delivery.dest == target_lapic.get_id()
                || (delivery.dest & BOCHS_APIC_BUS_ID_MASK) == BOCHS_APIC_BUS_ID_MASK
        }
    }

    fn deliver_ioapic_to_lapic(
        &mut self,
        delivery: crate::iodev::ioapic::PendingIoApicDelivery,
        target: usize,
    ) {
        self.deliver_lapic_bus_interrupt(
            target,
            delivery.vector,
            delivery.delivery_mode,
            delivery.trigger_mode,
        );
    }

    fn deliver_ioapic_to_lapics(
        &mut self,
        delivery: crate::iodev::ioapic::PendingIoApicDelivery,
    ) -> bool {
        let cpu_count = self.cpu_count();
        if delivery.delivery_mode == 1 {
            if delivery.dest_mode == 0 {
                return false;
            }

            if !self.cpu_ref(0).lapic.is_xapic() {
                let mut focus_target = None;
                for target in 0..cpu_count {
                    if self.cpu_ref(target).lapic.is_focus(delivery.vector) {
                        focus_target = Some(target);
                        break;
                    }
                }
                if let Some(target) = focus_target {
                    self.deliver_ioapic_to_lapic(delivery, target);
                    return true;
                }
            }

            let mut selected = None;
            for target in 0..cpu_count {
                if !self.ioapic_targets_cpu(delivery, target) {
                    continue;
                }
                let priority = {
                    let lapic = &self.cpu_ref(target).lapic;
                    if lapic.is_xapic() {
                        lapic.get_tpr()
                    } else {
                        lapic.get_apr()
                    }
                };
                if selected
                    .map(|(_, best_priority)| priority < best_priority)
                    .unwrap_or(true)
                {
                    selected = Some((target, priority));
                }
            }
            if let Some((target, _)) = selected {
                self.deliver_ioapic_to_lapic(delivery, target);
                return true;
            }
            return false;
        }

        let mut delivered = false;
        for target in 0..cpu_count {
            if self.ioapic_targets_cpu(delivery, target) {
                self.deliver_ioapic_to_lapic(delivery, target);
                delivered = true;
            }
        }
        delivered
    }

    fn apply_lapic_timer_request(
        &mut self,
        cpu_index: usize,
        timer_handle: Option<usize>,
        deactivate: bool,
        activate: Option<LocalApicTimerActivation>,
        _reactivate_from_previous_fire: bool,
    ) {
        if deactivate {
            if let Some(handle) = timer_handle {
                if let Err(e) = self.pc_system.deactivate_timer(handle) {
                    tracing::error!(
                        "CPU {cpu_index} LAPIC timer deactivate (handle {handle}) failed: {e:?}"
                    );
                }
            }
        }

        if let Some(activation) = activate {
            if let Some(handle) = timer_handle {
                if let Err(e) = self.pc_system.activate_timer_at_ticks(
                    handle,
                    activation.deadline_ticks,
                    false,
                ) {
                    tracing::error!(
                        "CPU {cpu_index} LAPIC timer activate (handle {handle}) failed: {e:?}"
                    );
                }
            }
            if activation.update_ticks_initial {
                let programmed_ticks = {
                    let lapic = &self.cpu_ref(cpu_index).lapic;
                    activation
                        .deadline_ticks
                        .saturating_sub(lapic.timer_period_ticks().unwrap_or(0))
                };
                self.cpu_mut_at(cpu_index)
                    .lapic
                    .set_ticks_initial(programmed_ticks);
            }
        }
    }

    /// Apply guest LAPIC timer programming at the SMP round boundary before
    /// advancing that round's virtual time. Bochs activates these timers at
    /// the register write; deferred Rust requests must retain the same epoch.
    pub(super) fn service_lapic_timer_requests(&mut self) {
        let ticks_now = self.pc_system.time_ticks();
        let cpu_count = self.cpu_count();
        let mut cursor = 0usize;
        while let Some(cpu_index) = self.lapic_work_mask.next_set(cursor, cpu_count) {
            cursor = cpu_index + 1;
            let has_request = {
                let lapic = &self.cpu_ref(cpu_index).lapic;
                lapic.timer_deactivate_request || lapic.timer_activate_request.is_some()
            };
            if !has_request {
                continue;
            }

            let (timer_handle, deactivate, activate) = {
                let cpu = self.cpu_mut_at(cpu_index);
                cpu.lapic.current_ticks = ticks_now;
                cpu.lapic.ticks_at_sync = ticks_now;
                cpu.lapic.cpu_ticks_at_sync = cpu.cpu_ticks();
                let timer_handle = cpu.lapic.timer_handle;
                let deactivate = cpu.lapic.timer_deactivate_request;
                cpu.lapic.timer_deactivate_request = false;
                let activate = cpu.lapic.timer_activate_request.take();
                (timer_handle, deactivate, activate)
            };
            self.apply_lapic_timer_request(cpu_index, timer_handle, deactivate, activate, false);
            self.refresh_cpu_masks(cpu_index);
        }
    }

    pub(super) fn service_lapic_local_events(&mut self) {
        let cpu_count = self.cpu_count();
        let mut cursor = 0usize;
        while let Some(cpu_index) = self.lapic_work_mask.next_set(cursor, cpu_count) {
            cursor = cpu_index + 1;
            while let Some(cpu_event) = self.cpu_mut_at(cpu_index).lapic.take_pending_cpu_event() {
                self.apply_lapic_cpu_event(cpu_index, Some(cpu_event));
            }

            while self.cpu_ref(cpu_index).lapic.timer_fired {
                let ticks_now = self.pc_system.time_ticks();
                let (timer_handle, deactivate, activate) = {
                    let cpu = self.cpu_mut_at(cpu_index);
                    cpu.lapic.current_ticks = ticks_now;
                    cpu.lapic.ticks_at_sync = ticks_now;
                    cpu.lapic.cpu_ticks_at_sync = cpu.cpu_ticks();
                    cpu.lapic.timer_fired = false;
                    cpu.lapic.diag_timer_fires += 1;
                    cpu.lapic.periodic(ticks_now);

                    if cpu.lapic.intr {
                        cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                    }

                    let timer_handle = cpu.lapic.timer_handle;
                    let deactivate = cpu.lapic.timer_deactivate_request;
                    cpu.lapic.timer_deactivate_request = false;
                    let activate = cpu.lapic.timer_activate_request.take();
                    (timer_handle, deactivate, activate)
                };

                self.apply_lapic_timer_request(cpu_index, timer_handle, deactivate, activate, true);

            }

            let ticks_now = self.pc_system.time_ticks();
            let (timer_handle, deactivate, activate, eoi_vector) = {
                let cpu = self.cpu_mut_at(cpu_index);
                cpu.lapic.current_ticks = ticks_now;
                cpu.lapic.ticks_at_sync = ticks_now;
                cpu.lapic.cpu_ticks_at_sync = cpu.cpu_ticks();

                if cpu.lapic.intr {
                    cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                }

                let timer_handle = cpu.lapic.timer_handle;
                let deactivate = cpu.lapic.timer_deactivate_request;
                cpu.lapic.timer_deactivate_request = false;
                let activate = cpu.lapic.timer_activate_request.take();
                let eoi_vector = cpu.lapic.pending_eoi_vector.take();
                (timer_handle, deactivate, activate, eoi_vector)
            };

            self.apply_lapic_timer_request(cpu_index, timer_handle, deactivate, activate, false);

            if let Some(vector) = eoi_vector {
                self.device_manager.irq.receive_eoi(vector);
            }
            self.refresh_cpu_masks(cpu_index);
        }
    }

    /// Commit all deferred machine effects after CPU/device raw borrows end.
    ///
    /// A CPU can queue work, but neither it nor a sibling observes the
    /// resulting machine state until this method has returned.
    ///
    /// Returns `true` when a reset was applied at this boundary. In that case
    /// every previously queued boundary effect and the caller's
    /// `elapsed_ticks` were discarded and virtual time did not advance: Bochs
    /// pc_system.cc bx_pc_system_c::Reset runs synchronously inside the
    /// triggering OUT, so nothing accrued before the reset is observable by
    /// the post-reset machine.
    /// Exact no-work test for a zero-elapsed scheduler boundary.
    ///
    /// True iff `service_scheduler_boundary(0)` would perform any state
    /// change. Every queue and latch the boundary drains is enumerated here —
    /// a false negative would strand queued device work, so any new boundary
    /// work source MUST be added to this predicate as well.
    ///
    /// Bochs main.cc's SMP round loop runs no per-slice device servicing at
    /// all (devices act only when `tickn` fires a timer), so skipping our
    /// no-op boundaries between slices is what keeps the SMP hot loop at
    /// comparable cost; the servicing itself remains exactly Bochs-ordered
    /// whenever any work exists.
    #[inline]
    fn scheduler_boundary_work_pending(&self) -> bool {
        // LAPIC bus IPIs, local events, and timer requests
        // (drain_lapic_bus / service_lapic_local_events /
        // service_lapic_timer_requests) — the mask is refreshed by
        // `refresh_cpu_masks` immediately after every CPU slice.
        self.lapic_work_mask.next_set(0, self.cpu_count()).is_some()
            // I/O-latched PIC/HRQ levels, boundary request, timer requests.
            || self.devices.has_pending_boundary_work()
            // Direct 8237 HRQ slot (producers outside I/O dispatch).
            || self.device_manager.dma.has_hrq_request()
            // A20, PAM/SMRAM/BAR re-registration, and reset requests.
            || self.device_manager.has_pending_machine_boundary()
            // IOAPIC deliveries deferred until the LAPIC bus is reachable
            // (sync_final_event_levels): enqueued by mid-slice I/O without
            // setting any request flag, so they must be checked directly.
            || self.device_manager.irq.has_pending_deliveries()
            // PIC edge bookkeeping awaiting collapse to a final level.
            || self.device_manager.irq.pic().irq_pending
            || self.device_manager.irq.pic().irq_cleared
            // Latched CPU events (sync_final_event_levels tail).
            || self.pc_system.intr_raised
            || self.pc_system.intr_cleared
            || self.pc_system.async_event_pending
            // Fired-timer dispatch loop.
            || self.pc_system.has_fired_timers()
            // Cross-CPU SMC invalidation (drain_pending_smc) — must reach
            // every sibling icache before another CPU runs (Bochs icache.cc
            // handleSMC).
            || self.memory.smc_has_pending()
            // HPET side effects queued from MMIO context (drain_hpet_pending).
            || self.device_manager.hpet.has_pending_work()
    }

    pub fn service_scheduler_boundary(&mut self, elapsed_ticks: u64) -> CpuResult<bool> {
        self.sync_vga_vertical_timer();

        // Bochs unmapped.cc port 0x8900: a completed "Shutdown" protocol sets
        // `bx_user_quit = 1` and BX_FATALs. Translate that guest request into
        // our run-loop stop flag — a graceful stop at the next boundary in
        // place of Bochs's immediate abort. Drained unconditionally (the flag
        // lives on `devices`, outside `scheduler_boundary_work_pending`'s
        // DeviceManager view). The cause is recorded alongside so the batch
        // boundary can report a guest power-off as such, rather than as a host
        // stop request; the flag itself is shared and cannot carry it.
        if self.devices.take_shutdown_request() {
            tracing::info!("port 0x8900 shutdown protocol complete — stopping emulation");
            self.stop_cause = crate::emulator::StopCause::GuestPowerOff;
            self.stop_flag
                .store(true, core::sync::atomic::Ordering::Relaxed);
        }

        // Bochs acpi.cc PM1_CNT SLP_EN with SLP_TYP=0 (S5 soft power off) sets
        // `bx_user_quit = 1` and BX_FATALs. Same treatment as port 0x8900: stop
        // the run loop gracefully instead of aborting. Drained unconditionally,
        // before the `had_work` gate, so it can never trip the apply/quiesce
        // convergence check on has_pending_machine_boundary.
        if core::mem::take(&mut self.device_manager.acpi.soft_off_pending) {
            tracing::info!("ACPI S5 soft power off — stopping emulation");
            self.stop_cause = crate::emulator::StopCause::GuestPowerOff;
            self.stop_flag
                .store(true, core::sync::atomic::Ordering::Relaxed);
        }

        // No-work fast path: when nothing is queued anywhere, every drain in
        // the prologue below is a no-op by construction, so skip straight to
        // the tick loop. Bochs main.cc's SMP round commit is exactly this: a
        // bare BX_TICKN with no device servicing attached. The epilogue after
        // the tick loop still runs unconditionally — its final-level
        // publication and mask refresh are state normalizations, not queue
        // drains, and callers rely on them independently of queued work.
        let had_work = self.scheduler_boundary_work_pending();
        if had_work {
        // Reset dominates every previously queued effect. Hardware requests
        // win over software requests from the same boundary.
        let reset_applied = match self.check_and_handle_resets() {
            Ok(applied) => applied,
            Err(error) => {
                tracing::error!("machine boundary reset handling failed: {error:?}");
                return Err(CpuError::MachineBoundaryFailed);
            }
        };

        if reset_applied {
            // Reset discarded all pre-reset LAPIC/device/timer work and
            // rearmed reset-time owners; servicing anything further here
            // (or ticking elapsed_ticks) would let pre-reset state leak into
            // the fresh machine — e.g. a rearmed timer firing before the
            // first instruction at the reset vector.
            #[cfg(test)]
            self.assert_cpu_masks_match_scan();
            return Ok(true);
        }

        // Bochs apic.cc apic_bus_deliver_smi(): an SMI raised by the ACPI
        // controller (OUT to SMI_CMD 0xB2 with APMC_EN set) goes to CPU 0.
        // Drained before the apply/quiesce loop below so the pending flag
        // never trips its has_pending_machine_boundary convergence check.
        if core::mem::take(&mut self.device_manager.acpi.smi_request_pending) {
            self.cpu_mut_at(0).deliver_smi();
        }

        // Source bus work before local control/EOI and captured-epoch timer
        // requests.
        self.drain_lapic_bus();
        self.service_lapic_local_events();
        self.service_lapic_timer_requests();
        self.drain_hpet_pending();

        // Apply the final 8237 HRQ level (Bochs pc_system.cc set_HRQ). The
        // I/O-dispatch copy covers CPU-issued port traffic; the direct DMA
        // slot covers producers outside I/O dispatch (tests driving set_drq,
        // device timer callbacks).
        let mut hrq_level = self.devices.take_hrq_level();
        if let Some(level) = self.device_manager.dma.take_hrq_request() {
            hrq_level = Some(level);
        }
        if let Some(level) = hrq_level {
            self.pc_system.set_hrq(level);
        }

        // Apply A20 and PCI/memory effects until no producer remains. Capture
        // both simultaneous A20 desires before changing either controller
        // mirror, then apply the established port92-then-keyboard order.
        let mut mapping_changed = false;
        let mut quiesced = false;
        for _ in 0..16 {
            let (port92_a20, keyboard_a20) = {
                let devices = &mut self.device_manager;
                let port92 = devices
                    .port92
                    .a20_change_pending
                    .then_some(devices.port92.a20_gate);
                let keyboard = devices
                    .keyboard
                    .a20_change_pending
                    .then_some(devices.keyboard.a20_enabled);
                devices.port92.a20_change_pending = false;
                devices.keyboard.a20_change_pending = false;
                (port92, keyboard)
            };
            if let Some(enabled) = port92_a20 {
                mapping_changed |= self.apply_a20_gate(enabled);
            }
            if let Some(enabled) = keyboard_a20 {
                mapping_changed |= self.apply_a20_gate(enabled);
            }

            match self.device_manager.apply_pending_machine_boundary(
                &mut self.devices,
                &mut self.memory,
            ) {
                Ok(effects) => mapping_changed |= effects.memory_mapping_changed,
                Err(error) => {
                    tracing::error!("machine boundary application failed: {error:?}");
                    self.invalidate_all_cpu_host_mappings();
                    return Err(CpuError::MachineBoundaryFailed);
                }
            }

            if !self.device_manager.has_pending_machine_boundary() {
                quiesced = true;
                break;
            }
        }
        if !quiesced {
            #[cfg(test)]
            tracing::debug!(
                "machine boundary failed to quiesce: platform={:?} rest={:?}",
                self.device_manager.pending,
                (
                    self.device_manager.port92.a20_change_pending,
                    self.device_manager.keyboard.a20_change_pending,
                    self.device_manager.port92.reset_request,
                    self.device_manager.keyboard.reset_requested,
                    self.device_manager.pci2isa.reset_request,
                )
            );
            self.invalidate_all_cpu_host_mappings();
            return Err(CpuError::MachineBoundaryFailed);
        }
        let a20_enabled = self.pc_system.get_enable_a20();
        self.device_manager.port92.a20_gate = a20_enabled;
        self.device_manager.keyboard.a20_enabled = a20_enabled;
        if mapping_changed {
            self.invalidate_all_cpu_host_mappings();
        }
        self.drain_device_timer_requests();
        } // had_work prologue
        // Step virtual time only to the earliest owner deadline, dispatch
        // every tied owner in registration order, then recompute. Callback
        // rearming cannot be skipped by a large tickn leap.
        let mut remaining = elapsed_ticks;
        let mut zero_time_passes = 0usize;
        loop {
            if self.pc_system.has_fired_timers() {
                self.dispatch_timer_fires();
                self.service_lapic_local_events();
                self.drain_device_timer_requests();
                zero_time_passes += 1;
                if zero_time_passes > 256 {
                    return Err(CpuError::UnsupportedCpuOperation {
                        operation: "scheduler timer callbacks failed to quiesce",
                    });
                }
                continue;
            }
            zero_time_passes = 0;
            if remaining == 0 {
                break;
            }

            // At least one tick: this loop must make progress even when a
            // deadline is already due, which the query reports as zero.
            let until_deadline = self
                .pc_system
                .ticks_to_next_timer_deadline()
                .map(|ticks| ticks.max(1))
                .unwrap_or(u64::MAX);
            let step = remaining.min(until_deadline).min(u64::from(u32::MAX));
            debug_assert_ne!(step, 0);
            self.pc_system.tickn(step as u32);
            remaining -= step;
        }

        self.drain_device_timer_requests();
        self.drain_pending_smc();
        self.sync_final_event_levels();
        #[cfg(test)]
        self.assert_cpu_masks_match_scan();
        Ok(false)
    }

    pub(super) fn apply_lapic_cpu_event(&mut self, target: usize, event: Option<LocalApicCpuEvent>) {
        let Some(event) = event else {
            return;
        };
        match event {
            // SMI / NMI / INIT only set an event bit — no memory access here;
            // the actual delivery happens at the target's next instruction
            // boundary in handle_async_event.
            LocalApicCpuEvent::Smi => self.cpu_mut_at(target).deliver_smi(),
            LocalApicCpuEvent::Nmi => self.cpu_mut_at(target).deliver_nmi(),
            LocalApicCpuEvent::Init => self.cpu_mut_at(target).deliver_init(),
            // deliver_sipi VMexits when the target is in VMX non-root
            // operation, and the exit can walk the VMEXIT MSR store/load
            // lists, so it runs on the target's execution context.
            LocalApicCpuEvent::Sipi(vector) => self.exec_ctx(target).deliver_sipi(vector),
        }
    }
    /// Rebuild CPU interrupt-level bits after snapshot restore without
    /// consuming any restored PIC, IOAPIC, LAPIC, or timer work queues.
    #[cfg(feature = "std")]
    pub(super) fn sync_restored_event_levels(&mut self) {
        let pic_asserted = self.device_manager.irq.int_pin_asserted()
            || self.device_manager.irq.pic().irq_pending
            || self.pc_system.intr_raised;
        if pic_asserted {
            self.cpu_mut().signal_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
        } else {
            self.cpu_mut().clear_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
        }

        for cpu_index in 0..self.cpu_count() {
            let cpu = self.cpu_mut_at(cpu_index);
            if cpu.lapic.intr || cpu.lapic.intr_pending {
                cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
            } else {
                cpu.clear_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
            }
        }
    }

    /// Synchronize final physical interrupt levels after all queue owners have
    /// committed. This is deliberately not a scheduler entry point.
    fn sync_final_event_levels(&mut self) {
        // Publish the physical PIC pin on every commit, not only when legacy
        // edge bookkeeping happens to be present. This restores the level
        // after CPU reset and prevents lost interrupt state.
        let asserted = self.device_manager.irq.int_pin_asserted();
        self.device_manager.irq.pic_mut().irq_pending = false;
        self.device_manager.irq.pic_mut().irq_cleared = false;
        if asserted {
            self.cpu_mut().signal_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
        } else {
            self.cpu_mut().clear_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
        }

        // The I/O APIC's levels are already current — the fabric moved them
        // when the lines did. What is left is routing its queued messages, in
        // registration order, into the LAPICs, which live on the CPUs and so
        // sit outside the fabric.
        let (deliveries, count) = self
            .device_manager
            .irq
            .ioapic_mut()
            .take_pending_deliveries();
        for &delivery in &deliveries[..count] {
            let mut delivery = delivery;
            if delivery.needs_pic_iac {
                // Only a snapshot written before the fabric existed can carry
                // an unresolved ExtINT message: the vector is read inside the
                // servicing scan now. Finishing the acknowledge here keeps
                // such an image bootable, and routes it through the one INTA
                // site like every other.
                delivery.vector = self.device_manager.irq.acknowledge();
                delivery.needs_pic_iac = false;
            }
            let done = self.deliver_ioapic_to_lapics(delivery);
            self.device_manager
                .irq
                .ioapic_mut()
                .complete_deferred_delivery(delivery, done);
        }

        self.drain_lapic_bus();
        self.service_lapic_local_events();
        let cpu_count = self.cpu_count();
        let mut cursor = 0usize;
        while let Some(cpu_index) = self.lapic_work_mask.next_set(cursor, cpu_count) {
            cursor = cpu_index + 1;
            let cpu = self.cpu_mut_at(cpu_index);
            if cpu.lapic.intr_pending {
                cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                cpu.lapic.intr_pending = false;
            }
            self.refresh_cpu_masks(cpu_index);
        }

        if self.pc_system.intr_raised {
            self.cpu_mut().signal_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
            self.pc_system.intr_raised = false;
        }
        if self.pc_system.intr_cleared {
            self.cpu_mut().clear_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
            self.pc_system.intr_cleared = false;
        }
        if self.pc_system.async_event_pending {
            self.cpu_mut().async_event = 1;
            self.pc_system.async_event_pending = false;
        }
        if self.cpu_count() != 0 {
            self.refresh_cpu_masks(0);
        }
    }

    /// Legacy public entry point: host/UI callers now join the same central
    /// zero-time commit used between CPU slices. A reset applied here needs
    /// no branch: the next batch starts at the reset vector.
    pub fn sync_event_flags(&mut self) {
        if let Err(error) = self.service_scheduler_boundary(0) {
            tracing::error!("scheduler boundary event synchronization failed: {error:?}");
        }
    }
}
