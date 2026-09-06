use crate::cpu::{instrumentation::Instrumentation};
#[cfg(feature = "alloc")]
use crate::{
    cpu::{cpu::CpuActivityState, BxCpuC},
    Error, Result,
};

#[cfg(feature = "std")]
use alloc::format;
#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};
#[cfg(feature = "alloc")]
use core::sync::atomic::Ordering;

use super::{Emulator, SliceEngine};
#[cfg(feature = "std")]
use super::status_ips_from_retired_instructions;

impl<'a, T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    fn total_cpu_icount(&self) -> u64 {
        (0..self.cpu_count()).fold(0u64, |total, cpu_index| {
            total.saturating_add(self.cpu_ref(cpu_index).icount)
        })
    }
}

impl<'a, T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    #[cfg(feature = "alloc")]
    /// Run emulator interactively with GUI event handling
    ///
    /// This method integrates CPU execution with GUI event processing:
    /// - Handles keyboard input from GUI
    /// - Updates GUI display periodically
    /// - Processes device interrupts
    /// - Executes CPU instructions in batches
    ///
    /// Returns the number of instructions executed, or an error.
    pub fn run_interactive(&mut self, max_instructions: u64) -> Result<u64> {
        self.prepare_run();

        // Verify VGA BIOS and IPL diagnostic ranges through block-aware RAM
        // copies; guest RAM is never borrowed as one flat slice.
        {
            let rom_bytes = self.peek_ram_at(0xC0000, 4);
            tracing::trace!(
                "VGA ROM check at 0xC0000: {:02X?} (expect [55, AA, ...])",
                rom_bytes
            );
            let ipl_count = self.peek_ram_at(0x9FF80, 2);
            let ipl0_type = self.peek_ram_at(0x9FF00, 2);
            tracing::trace!(
                "IPL count at 0x9FF80: {:02X?}; IPL0 type at 0x9FF00: {:02X?} (expect zeros before POST)",
                ipl_count,
                ipl0_type,
            );
            // Check total memory size
            tracing::trace!("Memory len={:#x}", self.memory.get_memory_len());
        }

        // Force initial GUI update to show initial state
        self.device_manager.vga.force_initial_update();
        self.update_gui(); // Force initial update

        let mut instructions_executed = 0u64;
        #[cfg(feature = "std")]
        let mut last_gui_update = std::time::Instant::now();
        #[cfg(feature = "std")]
        let mut last_ips_update = std::time::Instant::now();
        #[cfg(feature = "std")]
        let mut last_ips_instructions = self.total_cpu_icount();
        // MIPS terminal log: separate tracker fired every 5M retired instructions.
        // At 20 MIPS (active) fires every 250ms; at 40K IPS (idle) fires every ~125s.
        // This prevents flooding the terminal with "0.04 MIPS" lines during HLT idle.
        #[cfg(feature = "std")]
        let mut last_mips_log_update = std::time::Instant::now();
        #[cfg(feature = "std")]
        let mut last_mips_log_instructions = 0u64;
        // Bochs VGA timer fires every ~40ms (25 fps). Use same interval for display parity.
        #[cfg(feature = "std")]
        const GUI_UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(40);
        #[cfg(feature = "std")]
        const IPS_SHOW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        #[cfg(feature = "std")]
        const MIPS_LOG_INTERVAL: u64 = 50_000_000;

        // BENCHMARK-ONLY (temporary, mirrors the same patch in the Bochs bench
        // worktree): emit (icount, host_usec) samples so a guest boot can be
        // compared phase by phase against upstream instead of as one aggregate.
        // Sampling on the instruction axis, not emulated ticks -- ticks advance
        // during HLT and would credit idle time as throughput. Inert unless
        // RUSTY_BOX_BENCH_FILE is set; costs one u64 compare per batch.
        #[cfg(feature = "std")]
        let mut bench_sink = std::env::var("RUSTY_BOX_BENCH_FILE")
            .ok()
            .and_then(|path| std::fs::File::create(path).ok());
        #[cfg(feature = "std")]
        let bench_interval: u64 = std::env::var("RUSTY_BOX_BENCH_INTERVAL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(25_000_000);
        #[cfg(feature = "std")]
        let bench_start = std::time::Instant::now();
        #[cfg(feature = "std")]
        let mut bench_next: u64 = if bench_sink.is_some() {
            bench_interval
        } else {
            u64::MAX
        };
        #[cfg(feature = "std")]
        if let Some(sink) = bench_sink.as_mut() {
            use std::io::Write;
            writeln!(sink, "icount,host_usec,ticks").map_err(Error::Io)?;
        }

        const INSTRUCTION_BATCH_SIZE: u64 = 100_000;
        const PROGRESS_LOG_INTERVAL: u64 = 10_000_000;
        let mut next_progress_log: i64 = PROGRESS_LOG_INTERVAL as i64;

        tracing::trace!("Starting interactive execution loop");

        #[cfg(debug_assertions)]
        let mut last_rip: u64 = u64::MAX;
        #[cfg(debug_assertions)]
        let mut stuck_count: u32 = 0;
        #[cfg(debug_assertions)]
        let mut stuck_reported = false;
        // Counter for consecutive HLT+IF=0 zero-batches (transient recovery)
        let mut hlt_if0_count: u32 = 0;
        // The boot processor's counter when this run began, so the budget below
        // measures what this run retired rather than what the machine ever has.
        let icount_at_start = self.cpu_ref(0).icount;
        while instructions_executed < max_instructions && !self.stop_flag.load(Ordering::Relaxed) {
            // 1. Handle GUI events (keyboard/mouse/serial input) first.
            self.pump_gui_input();

            // 2. Execute CPU instructions in batches
            let remaining_instructions = max_instructions - instructions_executed;
            let batch_size = remaining_instructions.min(INSTRUCTION_BATCH_SIZE);
            let result = self.run_cpu_batch_with_strict_limit(batch_size, true);

            let _should_update_gui = match result {
                Ok(progress) => {
                    // The budget is named in instructions, so it is counted in
                    // instructions — read off the boot processor rather than
                    // taken from the batch, which answers in elapsed time once
                    // the machine has more than one processor.
                    let before = instructions_executed;
                    instructions_executed = self.cpu_ref(0).icount.saturating_sub(icount_at_start);
                    let executed = instructions_executed.saturating_sub(before);

                    // Reset HLT+IF=0 counter on any batch that advanced the
                    // machine, whichever unit it advanced in: an AP running
                    // while the boot processor is halted is progress too.
                    if !progress.stalled() {
                        hlt_if0_count = 0;
                    }

                    // Milestone progress print every 500K instructions
                    #[cfg(debug_assertions)]
                    if instructions_executed % 500_000 < INSTRUCTION_BATCH_SIZE {
                        tracing::trace!(
                            "[{}k instr] RIP={:#010x} CS={:#06x} mode={} batch_returned={} activity={:?}",
                            instructions_executed / 1000,
                            self.cpu_ref(0).rip(),
                            self.cpu_ref(0).get_cs_selector(),
                            self.cpu_ref(0).get_cpu_mode(),
                            executed,
                            self.cpu_ref(0).activity_state,
                        );
                    }
                    // Detect zero-return batches (HLT or stuck)
                    if progress.stalled() {
                        // HLT with IF=0: CPU is dead (panic or intentional halt)
                        // Use counter-based approach: only break after N consecutive
                        // zero-batch HLT+IF=0 cycles. This allows transient IF=0 states
                        // (e.g. kernel cli/hlt sequences before init scripts) to recover.
                        if matches!(
                            self.cpu_ref(0).activity_state,
                            CpuActivityState::Hlt
                                | CpuActivityState::Mwait
                                | CpuActivityState::MwaitIf
                        ) && !self.cpu_ref(0).interrupts_enabled()
                        {
                            hlt_if0_count += 1;
                            // Warn once at 1000 but DON'T break — match egui behavior.
                            // The egui path never exits on HLT+IF=0 and eventually the
                            // kernel recovers (timer/NMI wakes CPU). Breaking here would
                            // prevent headless Alpine from reaching modloop phase.
                            if hlt_if0_count == 1000 {
                                tracing::trace!(
                                    "[ZERO-BATCH] HLT/MWAIT with IF=0 for 1000 consecutive batches at RIP={:#x} CS={:#06x} activity={:?} — continuing (egui-match)",
                                    self.cpu_ref(0).rip(), self.cpu_ref(0).get_cs_selector(), self.cpu_ref(0).activity_state,
                                );
                            }
                        } else {
                            hlt_if0_count = 0;
                        }
                    }

                    // If CPU triple-faulted into shutdown, stop emulation loop
                    // Write reset diagnostics to file in debug builds; warn-log in release
                    #[cfg(feature = "std")]
                    fn log_reset(msg: &str) {
                        #[cfg(debug_assertions)]
                        {
                            use std::io::Write;
                            if let Ok(mut f) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open("reset_log.txt")
                            {
                                if let Err(error) = writeln!(f, "{}", msg) {
                                    tracing::warn!("reset_log.txt write failed: {error}");
                                }
                            }
                        }
                        tracing::warn!("{}", msg);
                    }

                    if self.cpu_ref(0).is_in_shutdown() {
                        #[cfg(feature = "std")]
                        log_reset(&format!(
                            "TRIPLE-FAULT SHUTDOWN at RIP={:#x} CS={:#06x} icount={}",
                            self.cpu_ref(0).rip(),
                            self.cpu_ref(0).get_cs_selector(),
                            self.cpu_ref(0).icount
                        ));
                        break;
                    }


                    // -- Progress tracking --
                    let current_rip = self.cpu_ref(0).rip();

                    // Log progress every 10M instructions (countdown-based)
                    next_progress_log -= executed as i64;
                    if next_progress_log <= 0 {
                        next_progress_log += PROGRESS_LOG_INTERVAL as i64;
                        tracing::debug!(
                            "Progress: {}M instructions, RIP={:#x}",
                            instructions_executed / 1_000_000,
                            current_rip
                        );
                    }

                    // vsprintf diagnostic removed (bug found and fixed: ADD AL,Ib operated on AH)

                    // Detailed EIP trace to track POST progression
                    // Log every batch in the critical PM→POST transition range
                    #[cfg(debug_assertions)]
                    if (440_000..480_000).contains(&instructions_executed) {
                        let ipl_count = self
                            .peek_ram_at(0x9FF80, 2)
                            .try_into()
                            .map(u16::from_le_bytes)
                            .unwrap_or(0);
                        let ipl0_type = self
                            .peek_ram_at(0x9FF00, 2)
                            .try_into()
                            .map(u16::from_le_bytes)
                            .unwrap_or(0);
                        tracing::trace!(
                            "EIP trace: {} instr, CS:IP={:#06x}:{:#06x}, mode={}, IPL_count={}, IPL0_type={}",
                            instructions_executed,
                            self.cpu_ref(0).get_cs_selector(),
                            current_rip,
                            self.cpu_ref(0).get_cpu_mode(),
                            ipl_count, ipl0_type,
                        );
                    }

                    // Detect stuck loop: RIP unchanged for many batches (debug only)
                    #[cfg(debug_assertions)]
                    {
                        if current_rip == last_rip {
                            stuck_count += 1;
                            if stuck_count >= 10 && !stuck_reported {
                                stuck_reported = true;
                                let bp = self.cpu_ref(0).bp() as usize;
                                let ss_base = self.cpu_ref(0).get_ss_base() as usize;
                                let bp_phys = ss_base + bp;
                                let ax = self.cpu_ref(0).eax() as u16;
                                let mem_peek = self.peek_ram_at(bp_phys, 8);
                                let bp2 = mem_peek
                                    .get(2..4)
                                    .and_then(|bytes| bytes.try_into().ok())
                                    .map(u16::from_le_bytes)
                                    .unwrap_or(0);
                                let bp4 = mem_peek
                                    .get(4..6)
                                    .and_then(|bytes| bytes.try_into().ok())
                                    .map(u16::from_le_bytes)
                                    .unwrap_or(0);
                                let bp6 = mem_peek
                                    .get(6..8)
                                    .and_then(|bytes| bytes.try_into().ok())
                                    .map(u16::from_le_bytes)
                                    .unwrap_or(0);
                                tracing::trace!(
                                    "STUCK at RIP={:#x} after {}k instructions, last I/O read: port={:#06x} value={:#x}, CS={:#06x} mode={}, BP={:#06x} AX={:#06x} [BP+2]={:#06x} [BP+4]={:#06x} [BP+6]={:#06x}",
                                    current_rip,
                                    instructions_executed / 1000,
                                    self.devices.last_io_read_port,
                                    self.devices.last_io_read_value,
                                    self.cpu_ref(0).get_cs_selector(),
                                    self.cpu_ref(0).get_cpu_mode(),
                                    bp, ax, bp2, bp4, bp6,
                                );
                            }
                        } else {
                            stuck_count = 0;
                            stuck_reported = false;
                            last_rip = current_rip;
                        }
                    }

                    // Drain Bochs-style port 0xE9 output (if any) and print it.
                    // This is useful for very early debug output before VGA is initialized.
                    #[cfg(feature = "std")]
                    {
                        let e9 = self.devices.take_port_e9_output();
                        if !e9.is_empty() {
                            use std::io::Write;
                            // Write to BIOS output file if configured, otherwise to stdout
                            if let Some(ref mut bios_file) = self.bios_output_file {
                                bios_file.write_all(&e9).ok();
                                bios_file.flush().ok();
                            } else {
                                let mut out = std::io::stdout();
                                out.write_all(&e9).ok();
                                out.flush().ok();
                            }
                        }
                    }
                    #[cfg(not(feature = "std"))]
                    {
                        // No host sink exists without `std`, so the debug
                        // console has nowhere to go — drain it anyway, or its
                        // ring backs up and starts dropping the oldest bytes
                        // silently.
                        let dropped = self.devices.drain_port_e9_output().count();
                        if dropped > 0 {
                            tracing::trace!("{dropped} debug-console bytes had no host sink");
                        }
                    }

                    // Advance virtual time (Bochs-like ticking).
                    // Required so PIT can generate IRQ0 and BIOS can progress past HLT waits.
                    if self.config.ips.per_second() != 0 {
                        if matches!(
                            self.cpu_ref(0).activity_state,
                            CpuActivityState::Hlt
                                | CpuActivityState::Mwait
                                | CpuActivityState::MwaitIf
                        ) && self.can_fast_forward_bsp_hlt()
                        {
                            // CPU is halted/mwait: advance virtual clock in 10-tick steps until an
                            // interrupt is pending. Matches Bochs handleWaitForEvent + BX_TICKN(10).
                            //
                            // When a GUI is attached AND the CPU is in protected mode: sleep once
                            // after the batch to synchronise virtual time to wall-clock time.
                            // This prevents the Linux console blank timer from firing ~360x early.
                            //
                            // Protected-mode-only: BIOS runs in real mode (mode=0) and its F12
                            // boot-wait HLTs should execute at full speed so the BIOS boots
                            // quickly. The kernel (mode=2) is what needs real-time throttling.
                            //
                            // We sleep ONCE per batch (not per iteration): on Windows,
                            // thread::sleep rounds up to ~15.6ms so per-iteration sleeps of 10µs
                            // would become 15,600ms per batch instead of 1:1.
                            //
                            // Without a GUI (headless): spin at full speed; the caller injects
                            // periodic keystrokes to keep the screen alive.
                            // Bochs handleWaitForEvent (event.cc): while(1) + BX_TICKN(10).
                            // Advances pc_system time (NOT icount) until interrupt fires.
                            // TSC reads pc_system.time_ticks(), so TSC advances during HLT
                            // without inflating icount.
                            // Safety cap: Bochs uses while(1) on a separate CPU thread.
                            // We cap at 100M ticks to yield for max_instructions/GUI checks.
                            // No icount inflation — TSC reads pc_system.time_ticks() directly.
                            // MwaitIf: wake on interrupt even when IF=0 (ECX[0]=1).
                            let mwait_if =
                                matches!(self.cpu_ref(0).activity_state, CpuActivityState::MwaitIf);
                            let mut hlt_budget = 0u64;
                            while hlt_budget < 100_000_000 {
                                // Service host input while halted so an idle
                                // (tickless) guest wakes promptly on a keypress,
                                // instead of stalling for the whole halt budget.
                                self.pump_gui_input();
                                if self.has_interrupt()
                                    && (self.cpu_ref(0).interrupts_enabled() || mwait_if)
                                {
                                    break;
                                }
                                if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                self.service_lapic_local_events();
                                if self.cpu_ref(0).lapic.intr
                                    && (self.cpu_ref(0).interrupts_enabled() || mwait_if)
                                {
                                    self.cpu_mut()
                                        .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                                    break;
                                }
                                // 2. Advance halted virtual time in a device-friendly quantum.
                                let step = self.hlt_wait_step_ticks();
                                if self.service_scheduler_boundary(u64::from(step))? {
                                    // Reset: stop advancing time; the CPU is
                                    // Active at the reset vector.
                                    break;
                                }
                                hlt_budget += u64::from(step);
                                if !self.can_fast_forward_bsp_hlt() {
                                    break;
                                }
                            }

                            // If LAPIC has a pending interrupt, signal CPU
                            if self.cpu_ref(0).lapic_has_intr() {
                                self.cpu_mut()
                                    .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                            }

                            // Tight MWAIT loop: process multiple wake→execute→MWAIT
                            // cycles without returning to the outer loop. This matches
                            // Bochs's dedicated CPU thread which never yields to GUI
                            // between MWAIT wakes. Budget: 15ms wall-clock.
                            #[cfg(feature = "std")]
                            let mwait_wall_start = std::time::Instant::now();
                            #[cfg(feature = "std")]
                            let mwait_wall_budget = std::time::Duration::from_millis(15);
                            loop {
                                #[cfg(feature = "std")]
                                if mwait_wall_start.elapsed() >= mwait_wall_budget {
                                    break;
                                }
                                if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                // Deliver PIC interrupt if pending
                                if self.device_manager.has_interrupt()
                                    && self.cpu_ref(0).get_b_if() != 0
                                    && !self.cpu_ref(0).interrupts_inhibited(0x01)
                                    // Priority-4 debug traps come first —
                                    // see the main loop's injection site.
                                    && !self.cpu_ref(0).debug_trap_pending()
                                {
                                    let vec = self.iac();
                                    // SAFETY: see borrow_memory_for_cpu / inject_interrupt
                                    if let Err(e) = self.inject_interrupt(vec) {
                                        tracing::warn!(
                                            "PIC interrupt injection (vector {vec:#04x}) failed: {e:?}"
                                        );
                                    }
                                }
                                // Run CPU batch — handle_async_event inside cpu_loop_n
                                // will process LAPIC events and wake from MWAIT.
                                // Don't check activity_state here — LAPIC uses signal_event
                                // which sets async_event but doesn't change activity_state
                                // until handle_async_event runs inside the CPU loop.
                                let remaining_instructions =
                                    max_instructions.saturating_sub(instructions_executed);
                                let batch2 = remaining_instructions.min(INSTRUCTION_BATCH_SIZE);
                                if batch2 == 0 {
                                    break;
                                }
                                let r2 = self.run_cpu_batch_with_strict_limit(batch2, true);
                                if let Ok(progress2) = r2 {
                                    instructions_executed =
                                        self.cpu_ref(0).icount.saturating_sub(icount_at_start);
                                    if !self.batch_advanced_pc_system {
                                        self.advance_pc_system_after_cpu_ticks(progress2.count())?;
                                    }
                                } else {
                                    break;
                                }
                                // If CPU re-entered MWAIT, advance time again
                                if !matches!(
                                    self.cpu_ref(0).activity_state,
                                    CpuActivityState::Hlt
                                        | CpuActivityState::Mwait
                                        | CpuActivityState::MwaitIf
                                ) {
                                    break; // CPU is active — return to outer loop
                                }
                                // HLT loop: Bochs handleWaitForEvent advances BX_TICKN(10).
                                let mwait_if2 =
                                    matches!(self.cpu_ref(0).activity_state, CpuActivityState::MwaitIf);
                                let mut hlt2 = 0u64;
                                while hlt2 < 100_000_000 {
                                    // Keep host input responsive during MWAIT idle.
                                    self.pump_gui_input();
                                    if self.has_interrupt()
                                        && (self.cpu_ref(0).interrupts_enabled() || mwait_if2)
                                    {
                                        break;
                                    }
                                    self.service_lapic_local_events();
                                    if self.cpu_ref(0).lapic.intr
                                        && (self.cpu_ref(0).interrupts_enabled() || mwait_if2)
                                    {
                                        self.cpu_mut()
                                            .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                                        break;
                                    }
                                    let step = self.hlt_wait_step_ticks();
                                    if self.service_scheduler_boundary(u64::from(step))? {
                                        // Reset: stop advancing time; the CPU
                                        // is Active at the reset vector.
                                        break;
                                    }
                                    hlt2 += u64::from(step);
                                    if !self.can_fast_forward_bsp_hlt() {
                                        break;
                                    }
                                }
                                if self.cpu_ref(0).lapic_has_intr() {
                                    self.cpu_mut()
                                        .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                                }
                            }
                        }
                    }

                    // Drive pc_system timers via Bochs-exact tickn() mechanism.
                    if !self.batch_advanced_pc_system {
                        self.advance_pc_system_after_cpu_ticks(executed)?;
                    }


                    // Log batch sizes and check if timer ticking works
                    #[cfg(debug_assertions)]
                    if instructions_executed < 5 * INSTRUCTION_BATCH_SIZE
                        || instructions_executed % 100_000 < INSTRUCTION_BATCH_SIZE
                    {
                        let pit_c0_count = self.device_manager.pit.counters[0].count;
                        // Read the BDA timer tick counter through a fixed-size
                        // block-aware copy.
                        let bda_ticks = self
                            .peek_ram_at(0x046C, 4)
                            .as_slice()
                            .try_into()
                            .map(u32::from_le_bytes)
                            .unwrap_or(0);
                        tracing::trace!("BATCH-DIAG: executed={}, total={}k, RIP={:#x}, PIT_count={}, activity={:?}, BDA_ticks={}",
                            executed, instructions_executed / 1000, self.cpu_ref(0).rip(), pit_c0_count,
                            self.cpu_ref(0).activity_state, bda_ticks);
                    }

                    // Periodic interrupt-chain diagnostic (every ~1M instructions)
                    #[cfg(debug_assertions)]
                    if instructions_executed % 1_000_000 < INSTRUCTION_BATCH_SIZE {
                        let has_int = self.has_interrupt();
                        let if_flag = self.cpu_ref(0).get_b_if();
                        let rip = self.cpu_ref(0).rip();
                        let pit_c0 = &self.device_manager.pit.counters[0];
                        tracing::trace!(
                            "IRQ-DIAG: {}M instr, RIP={:#x}, IF={}, has_int={}, PIC_imr={:#04x}, PIC_irr={:#04x}, PIT_c0: mode={:?} inlatch={} count={} count_written={} gate={} output={}",
                            instructions_executed / 1_000_000,
                            rip,
                            if_flag,
                            has_int,
                            self.device_manager.irq.pic().master.imr,
                            self.device_manager.irq.pic().master.irr,
                            self.device_manager.pit.counters[0].mode,
                            pit_c0.inlatch,
                            pit_c0.count,
                            pit_c0.count_written,
                            pit_c0.gate,
                            pit_c0.output,
                        );
                    }

                    // Deliver pending PIC interrupts to the CPU (Bochs-like).
                    // Only use PIC path — LAPIC interrupts are delivered via
                    // handleAsyncEvent() through the CPU event system.
                    if self.device_manager.has_interrupt()
                        && self.cpu_ref(0).get_b_if() != 0
                        && !self.cpu_ref(0).interrupts_inhibited(0x01)
                        // BX_INHIBIT_INTERRUPTS
                        //
                        // Bochs event.cc handleAsyncEvent delivers Priority-4
                        // traps on the previous instruction (TF single-step,
                        // data/IO and code breakpoints) BEFORE Priority-5
                        // external interrupts. This path implements only
                        // Priority 5, and interrupt() unconditionally clears
                        // debug_trap, so injecting here while a #DB is pending
                        // silently destroyed it. Deferring by one boundary
                        // keeps the interrupt latched in the PIC (iac() is not
                        // called) and lets the CPU deliver #DB first.
                        && !self.cpu_ref(0).debug_trap_pending()
                    {
                        let vector = self.iac();

                        // Temporarily wire the memory bus so the interrupt path can
                        // read IVT/IDT and push stack frames correctly.
                        // SAFETY: see borrow_memory_for_cpu / inject_interrupt
                        let inject_result = self.inject_interrupt(vector);

                        match &inject_result {
                            Ok(()) => {
                                tracing::trace!(
                                    "INT-INJECT: OK! activity_after={:?}, RIP={:#x}",
                                    self.cpu_ref(0).activity_state,
                                    self.cpu_ref(0).rip()
                                );
                            }
                            Err(e) => {
                                tracing::error!("INT-INJECT: FAILED: {:?}", e);
                                return Err(Error::Cpu(inject_result.unwrap_err()));
                            }
                        }
                    }

                    // Progress logging removed per user request

                    // 4. Check if GUI should be updated
                    #[cfg(feature = "std")]
                    let should_update = {
                        // Update when text is dirty, or periodically to catch any missed updates
                        let text_dirty = self.device_manager.vga.is_text_dirty();
                        let time_since_update = last_gui_update.elapsed();
                        // Update if text changed OR periodically (like Bochs timer-based updates)
                        let should_update = text_dirty || time_since_update >= GUI_UPDATE_INTERVAL;
                        // Update timestamp if we're going to update
                        if should_update {
                            last_gui_update = std::time::Instant::now();
                        }
                        should_update
                    };
                    #[cfg(not(feature = "std"))]
                    let should_update = false;
                    should_update
                }
                Err(e) => {
                    tracing::error!("CPU execution error: {:?}", e);
                    tracing::trace!("[Emulator] ERROR: {:?}", e);
                    return Err(Error::Cpu(e));
                }
            };

            // Drain serial port output every batch for responsive serial console.
            // Previously gated by should_update_gui (100ms) — now immediate.
            {
                let serial_bytes: Vec<u8> = self.device_manager.drain_serial_tx(0).collect();
                if !serial_bytes.is_empty() {
                    if let Some(ref gui) = self.gui {
                        let text = String::from_utf8_lossy(&serial_bytes);
                        gui.append_serial_log(&text);
                    }
                    // Always write serial output to stdout for headless/terminal visibility
                    #[cfg(feature = "std")]
                    {
                        use std::io::Write;
                        // A closed or redirected stdout must not stop the
                        // guest, but it does mean the mirror is now lying —
                        // say so once per failed write rather than never.
                        let mirrored = std::io::stdout()
                            .write_all(serial_bytes.as_slice())
                            .and_then(|()| std::io::stdout().flush());
                        if let Err(error) = mirrored {
                            tracing::warn!("serial stdout mirror failed: {error}");
                        }
                    }
                }
            }

            // BENCHMARK-ONLY (temporary): see the bench_sink setup above.
            #[cfg(feature = "std")]
            {
                let retired = self.total_cpu_icount();
                if retired >= bench_next {
                    if let Some(sink) = bench_sink.as_mut() {
                        use std::io::Write;
                        bench_next = retired + bench_interval;
                        let ticks = self.pc_system.time_ticks();
                        let rip = self.cpu_ref(0).rip();
                        writeln!(
                            sink,
                            "{retired},{},{ticks},{rip:x}",
                            bench_start.elapsed().as_micros()
                        )
                        .map_err(Error::Io)?;
                        let mut vec_error = None;
                        crate::vec_diag::snapshot(|index, count| {
                            if vec_error.is_none() {
                                if let Err(error) =
                                    writeln!(sink, "V,{retired},{index},{count}")
                                {
                                    vec_error = Some(error);
                                }
                            }
                        });
                        if let Some(error) = vec_error {
                            return Err(Error::Io(error));
                        }
                        sink.flush().map_err(Error::Io)?;
                    }
                }
            }

            // Update GUI after CPU execution
            #[cfg(feature = "std")]
            if _should_update_gui {
                self.update_gui();
            }

            #[cfg(feature = "std")]
            {
                // Update IPS: show_ips() every 1 real second (keeps egui status bar responsive).
                // This is retired CPU instructions per real second across all configured CPUs.
                // HLT/timer wait ticks are not CPU throughput and can sprint during firmware idle
                // loops, so they are not shown.
                let ips_elapsed = last_ips_update.elapsed();
                if ips_elapsed >= IPS_SHOW_INTERVAL {
                    let current_icount = self.total_cpu_icount();
                    let ips = status_ips_from_retired_instructions(
                        last_ips_instructions,
                        current_icount,
                        ips_elapsed,
                    );
                    last_ips_instructions = current_icount;
                    last_ips_update = std::time::Instant::now();
                    if let Some(ref mut gui) = self.gui {
                        gui.show_ips(ips);
                    }
                }
            }
            #[cfg(feature = "std")]
            {
                // Print MIPS terminal line every 50M instructions (~5s at 9 MIPS).
                if instructions_executed / MIPS_LOG_INTERVAL
                    > last_mips_log_instructions / MIPS_LOG_INTERVAL
                {
                    let log_elapsed = last_mips_log_update.elapsed();
                    let log_delta = instructions_executed - last_mips_log_instructions;
                    let mips = if log_elapsed.as_secs_f64() > 0.001 {
                        (log_delta as f64 / log_elapsed.as_secs_f64()) / 1_000_000.0
                    } else {
                        0.0
                    };
                    last_mips_log_instructions = instructions_executed;
                    last_mips_log_update = std::time::Instant::now();
                    tracing::debug!(
                        target: "mips",
                        "[{:>6}M instr] {:>6.2} MIPS  RIP={:#010x}  CS={:#06x}  mode={}",
                        instructions_executed / 1_000_000,
                        mips,
                        self.cpu_ref(0).rip(),
                        self.cpu_ref(0).get_cs_selector(),
                        self.get_cpu_mode_str(),
                    );
                }
            }


            // 6. Check if we should exit (e.g., shutdown requested)
            // TODO: Add shutdown flag check
        }

        tracing::trace!(
            "Interactive execution completed: {} instructions",
            instructions_executed
        );

        #[cfg(feature = "profiling")]
        {
            // Print perf summary to stderr (only for large batches, not sub-batches)
            if instructions_executed >= 1_000_000 {
                let pi = self.cpu_ref(0).perf_instructions;
                let tlb_h = self.cpu_ref(0).perf_tlb_hit;
                let tlb_m = self.cpu_ref(0).perf_tlb_miss;
                let pw = self.cpu_ref(0).perf_page_walk;
                let ic_m = self.cpu_ref(0).perf_icache_miss;
                let pf = self.cpu_ref(0).perf_prefetch;
                let tlb_total = tlb_h + tlb_m;
                let tlb_pct = if tlb_total > 0 {
                    tlb_h as f64 / tlb_total as f64 * 100.0
                } else {
                    0.0
                };
                // cpu_ticks = instruction count plus the fast-REP tick surplus
                // (Bochs BX_TICK1-per-instruction + BX_TICKN time domain).
                let bochs_ticks = self.cpu_ref(0).cpu_ticks();
                tracing::debug!("[PERF] dispatches={pi} bochs_ticks={bochs_ticks} tlb_hit={tlb_h} tlb_miss={tlb_m} tlb_hit%={tlb_pct:.2}% page_walks={pw}");
            }
        }

        Ok(instructions_executed)
    }
}
