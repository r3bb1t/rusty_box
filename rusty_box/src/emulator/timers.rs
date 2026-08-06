use crate::{
    cpu::{
        instrumentation::Instrumentation,
    },
    iodev::{
        devices::DeviceManager, DeviceTimerOwner, TimerRequest,
    },
    pc_system::TimerOwner, Result,
};

use super::Emulator;
#[cfg(feature = "std")]
use super::SLOWDOWN_QUANTUM_USEC;

impl<'a, T: Instrumentation> Emulator<'a, T> {
    /// Initialize the emulator
    ///
    /// This runs the full initialization sequence from Bochs main.cc (bx_init_hardware):
    /// 1. PC system initialization (timers, IPS) - line 1201
    /// 2. Memory initialization - line 1312
    /// 3. BIOS load - line 1315-1316 (done via load_bios() after this call)
    /// 4. Optional ROM load - line 1319-1325 (done via load_optional_rom())
    /// 5. Optional RAM load - line 1328-1334 (done via load_ram())
    /// 6. CPU initialization - line 1337
    /// 7. CPU sanity checks - line 1338
    /// 8. CPU register state - line 1339
    /// 9. Device initialization - line 1353
    /// 10. PC system register state - line 1356
    /// 11. Device register state - line 1357
    /// 12. Reset - line 1363 (done via reset() after this call)
    /// 13. GUI signal handlers - line 1383 (done via init_gui() or after reset)
    /// 14. Start timers - line 1384 (done in reset())
    ///
    /// After this, call `load_bios()` to load a BIOS image, then `reset()` and `run()`.
    ///
    /// **IMPORTANT**: For correct BIOS initialization sequence matching original Bochs,
    /// use `init_memory()` + `load_bios()` + `init_cpu_and_devices()` instead of this method.
    /// See main.cc for the correct sequence.
    pub(super) fn register_timer_owners(&mut self) -> Result<()> {
        let pit = self
            .pc_system
            .register_timer(TimerOwner::Pit, 0, false, false, "PIT")?;
        self.device_manager.pit.set_timer_handle(pit);

        let keyboard = self.pc_system.register_timer(
            TimerOwner::Keyboard,
            0,
            false,
            false,
            "keyboard",
        )?;
        self.device_manager.keyboard.set_timer_handle(keyboard);

        // Bochs hpet.cc init(): one one-shot timer per comparator with the
        // comparator index as its param ("hpet").
        for index in 0..crate::iodev::hpet::HPET_NUM_TIMERS {
            let handle =
                self.pc_system
                    .register_timer(TimerOwner::Hpet(index), 0, false, false, "hpet")?;
            self.device_manager.hpet.timer_handles[index] = Some(handle);
        }

        // Bochs vgacore.cc registers a continuous "vga vertical timer" at the
        // vertical-retrace period; it latches the frame's CRTC start address and
        // re-anchors the 0x3DA phase. Armed once the retrace timing is known.
        self.vga_vertical_timer_handle = Some(self.pc_system.register_timer(
            TimerOwner::VgaVertical,
            0,
            false,
            false,
            "vga vertical timer",
        )?);

        self.device_manager.cmos.periodic_timer_handle = Some(self.pc_system.register_timer(
            TimerOwner::CmosPeriodic,
            0,
            false,
            false,
            "CMOS periodic",
        )?);
        self.device_manager.cmos.one_second_timer_handle = Some(self.pc_system.register_timer(
            TimerOwner::CmosOneSecond,
            0,
            false,
            false,
            "CMOS second",
        )?);
        self.device_manager.cmos.uip_timer_handle = Some(self.pc_system.register_timer(
            TimerOwner::CmosUip,
            0,
            false,
            false,
            "CMOS UIP",
        )?);

        self.device_manager.acpi.overflow_timer_handle = Some(self.pc_system.register_timer(
            TimerOwner::AcpiPmOverflow,
            0,
            false,
            false,
            "ACPI overflow",
        )?);

        for port_index in 0..self.device_manager.serial.configured_port_count() {
            let handle = self.pc_system.register_timer(
                TimerOwner::SerialFifo(port_index),
                0,
                false,
                false,
                "serial FIFO",
            )?;
            self.device_manager
                .serial
                .set_fifo_timer_handle(port_index, Some(handle));
            let tx_handle = self.pc_system.register_timer(
                TimerOwner::SerialTx(port_index),
                0,
                false,
                false,
                "serial TX",
            )?;
            self.device_manager
                .serial
                .set_tx_timer_handle(port_index, Some(tx_handle));
        }

        for (owner, channel) in [
            (TimerOwner::PciIdeCh0, 0usize),
            (TimerOwner::PciIdeCh1, 1usize),
        ] {
            let handle = self
                .pc_system
                .register_timer(owner, 0, false, false, "PIIX IDE")?;
            self.device_manager.pci_ide.bmdma[channel].timer_index = Some(handle);
        }

        // Bochs harddrv.cc init registers one "HD/CD seek" timer per
        // configured drive (param = channel<<1 | device). Media may attach
        // after device init here, so all four slots are registered up front;
        // a slot whose drive stays absent simply never activates.
        for channel in 0..2usize {
            for device in 0..2usize {
                let param = (channel << 1) | device;
                let handle = self.pc_system.register_timer(
                    TimerOwner::HdSeek(param),
                    0,
                    false,
                    false,
                    "HD/CD seek",
                )?;
                self.device_manager.harddrv.seek_timer_handles[channel][device] = Some(handle);
            }
        }

        for cpu_index in 0..self.cpu_count() {
            let handle = self.pc_system.register_timer(
                TimerOwner::Lapic(cpu_index),
                0,
                false,
                false,
                "lapic",
            )?;
            self.cpu_mut_at(cpu_index).lapic.timer_handle = Some(handle);
        }

        #[cfg(feature = "std")]
        if self.config.sync_slowdown {
            let handle = self.pc_system.register_timer(
                TimerOwner::Slowdown,
                0,
                false,
                false,
                "slowdown",
            )?;
            self.slowdown_timer.initialize(
                handle,
                self.pc_system.time_usec(),
                std::time::Instant::now(),
            );
            self.pc_system
                .activate_timer_usec(handle, SLOWDOWN_QUANTUM_USEC as u32, false)?;
        }
        let current_ticks = self.pc_system.time_ticks();
        self.devices.request_timer_after_usec(
            DeviceTimerOwner::Pit,
            current_ticks,
            self.device_manager.pit.next_event_usec(),
        );
        self.devices
            .apply_cmos_timer_sync(current_ticks, self.device_manager.cmos.timer_sync());
        self.drain_device_timer_requests();
        Ok(())
    }

    pub(super) fn rearm_device_timers_after_hardware_reset(&mut self) {
        let current_ticks = self.pc_system.time_ticks();
        for owner in [
            DeviceTimerOwner::Pit,
            DeviceTimerOwner::CmosPeriodic,
            DeviceTimerOwner::CmosOneSecond,
            DeviceTimerOwner::CmosUip,
            DeviceTimerOwner::AcpiPmOverflow,
            DeviceTimerOwner::PciIdeCh0,
            DeviceTimerOwner::PciIdeCh1,
        ] {
            self.devices.request_timer(owner, TimerRequest::Deactivate);
        }
        for port_index in 0..self.device_manager.serial.configured_port_count() {
            self.devices.request_timer(
                DeviceTimerOwner::SerialFifo(port_index),
                TimerRequest::Deactivate,
            );
            self.devices.request_timer(
                DeviceTimerOwner::SerialTx(port_index),
                TimerRequest::Deactivate,
            );
        }

        self.devices.request_timer_after_usec(
            DeviceTimerOwner::Pit,
            current_ticks,
            self.device_manager.pit.next_event_usec(),
        );
        // Bochs keyboard.cc init(): the 8042 timer is CONTINUOUS at the
        // serial_delay period and never stops; (re)start it here where the
        // IPS-based tick conversion is valid.
        if let Some(handle) = self.device_manager.keyboard.timer_handle() {
            if let Err(error) = self.pc_system.activate_timer_usec(
                handle,
                crate::iodev::keyboard::KBD_SERIAL_DELAY_USEC,
                true,
            ) {
                tracing::error!("failed to start the 8042 serial-delay timer: {error:?}");
            }
        }
        // Apply the timer-owner delta the CMOS produced during its own reset
        // (stashed by DeviceManager::reset because it can't reach the timers);
        // fall back to a fresh derivation if reset didn't run this path.
        let cmos_sync = self
            .device_manager
            .cmos_reset_timer_sync
            .take()
            .unwrap_or_else(|| self.device_manager.cmos.timer_sync());
        self.devices
            .apply_cmos_timer_sync(current_ticks, cmos_sync);
        self.devices.request_timer_after_usec(
            DeviceTimerOwner::AcpiPmOverflow,
            current_ticks,
            self.device_manager.acpi.overflow_delay_usec(current_ticks),
        );
        self.drain_device_timer_requests();
        // Bochs hpet.cc reset() queued comparator deactivations and the
        // PIT/RTC pin re-enables — apply them to the fresh machine.
        self.drain_hpet_pending();
    }

    #[inline]
    pub(super) fn advance_pc_system_after_cpu_ticks(&mut self, ticks: u64) {
        // On reset the boundary discards `ticks` itself; execution resumes at
        // the reset vector, so both outcomes continue identically here.
        if let Err(error) = self.service_scheduler_boundary(ticks) {
            tracing::error!("scheduler tick commit failed: {error:?}");
        }
    }

    /// Dispatch timer fires accumulated by `pc_system.tickn()`.
    ///
    pub fn dispatch_timer_fires(&mut self) {
        let (owners, counts, count) = self.pc_system.take_fired_timers();
        let current_ticks = self.pc_system.time_ticks();
        let ips = u64::from(self.config.ips);
        for entry in 0..count {
            match owners[entry] {
                TimerOwner::NullTimer => {}
                TimerOwner::VgaVertical => {
                    // Bochs vgacore.cc vertical_timer(): latch the start address
                    // for the frame and re-anchor the retrace phase. Coalesced —
                    // only the newest retrace matters.
                    let now_usec = if ips > 0 {
                        (current_ticks as u128 * 1_000_000 / ips as u128) as u64
                    } else {
                        0
                    };
                    self.device_manager.vga.vertical_timer(now_usec);
                }
                TimerOwner::PciIdeCh0 => {
                    for _ in 0..counts[entry] {
                        let pins_ptr = self.tlb_pins().as_ptr();
                        let pins_len = self.tlb_pins().len();
                        let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
                        self.device_manager
                            .pci_ide_timer(0, &mut self.pc_system, &mut self.memory, pins);
                    }
                }
                TimerOwner::PciIdeCh1 => {
                    for _ in 0..counts[entry] {
                        let pins_ptr = self.tlb_pins().as_ptr();
                        let pins_len = self.tlb_pins().len();
                        let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
                        self.device_manager
                            .pci_ide_timer(1, &mut self.pc_system, &mut self.memory, pins);
                    }
                }
                TimerOwner::HdSeek(param) => {
                    // Bochs harddrv.cc seek_timer — one-shot; the seek deadline
                    // completes the read command (DRQ/IRQ or BM-DMA start).
                    for _ in 0..counts[entry] {
                        let crate::iodev::devices::DeviceManager {
                            harddrv,
                            pic,
                            pci_ide,
                            ..
                        } = &mut self.device_manager;
                        harddrv.seek_timer(param as u8, pic, pci_ide);
                    }
                }
                TimerOwner::Pit => {
                    for _ in 0..counts[entry] {
                        let callback = self
                            .device_manager
                            .pit
                            .timer_callback(current_ticks, ips);
                        // Bochs pit.cc irq_handler: the HPET legacy-mode gate
                        // drops OUT transitions before they reach the PIC.
                        let rising = if self.device_manager.pit.irq_enabled {
                            DeviceManager::replay_pit_irq0_events(
                                callback.irq0_transitions,
                                callback.irq0_level,
                                &mut self.device_manager.pic,
                            )
                        } else {
                            0
                        };
                        if rising != 0 {
                            self.device_manager.diag_pit_fires += u64::from(rising);
                        }
                        self.devices.request_timer_after_usec(
                            DeviceTimerOwner::Pit,
                            current_ticks,
                            callback.rearm_usec,
                        );
                    }
                }
                TimerOwner::Hpet(index) => {
                    // Bochs hpet.cc timer_handler → hpet_timer(): runs in
                    // emulator context, so the queued IRQ edges and the
                    // comparator re-arm drain immediately.
                    for _ in 0..counts[entry] {
                        let ips = self.pc_system.ips();
                        self.device_manager.hpet.set_now(current_ticks, ips);
                        self.device_manager.hpet.timer_fired(index);
                        self.drain_hpet_pending();
                    }
                }
                TimerOwner::Keyboard => {
                    // Bochs keyboard.cc timer_handler: the continuous
                    // serial-delay timer runs periodic(1) every fire and
                    // raises whatever IRQs the controller latched since the
                    // previous fire. No rearm — pc_system reloads the period.
                    for _ in 0..counts[entry] {
                        let irq_mask = self.device_manager.keyboard.timer_callback();
                        if irq_mask & 0x01 != 0 {
                            self.device_manager.pic.raise_irq(1);
                        }
                        if irq_mask & 0x02 != 0 {
                            self.device_manager.pic.raise_irq(12);
                        }
                    }
                }
                TimerOwner::CmosPeriodic => {
                    for _ in 0..counts[entry] {
                        self.device_manager.cmos.periodic_timer();
                    }
                    if self.device_manager.cmos.check_irq8() {
                        self.device_manager.pic.raise_irq(8);
                    }
                }
                TimerOwner::CmosOneSecond => {
                    for _ in 0..counts[entry] {
                        if self.device_manager.cmos.one_second_timer() {
                            self.devices.request_timer_after_usec(
                                DeviceTimerOwner::CmosUip,
                                current_ticks,
                                Some(244),
                            );
                        }
                    }
                }
                TimerOwner::CmosUip => {
                    for _ in 0..counts[entry] {
                        self.device_manager.cmos.uip_timer();
                    }
                    if self.device_manager.cmos.check_irq8() {
                        self.device_manager.pic.raise_irq(8);
                    }
                }
                TimerOwner::AcpiPmOverflow => {
                    for _ in 0..counts[entry] {
                        let delay = self.device_manager.acpi.overflow_timer(current_ticks);
                        self.devices.request_timer_after_usec(
                            DeviceTimerOwner::AcpiPmOverflow,
                            current_ticks,
                            delay,
                        );
                    }
                    if self.device_manager.acpi.irq9_level {
                        self.device_manager.pic.raise_irq(9);
                    } else {
                        self.device_manager.pic.lower_irq(9);
                    }
                }
                TimerOwner::SerialFifo(port_index) => {
                    for _ in 0..counts[entry] {
                        self.device_manager.serial.fifo_timer_fired(port_index);
                    }
                    for (irq, raise) in self.device_manager.serial.take_pending_irqs() {
                        if raise {
                            self.device_manager.pic.raise_irq(irq);
                        } else {
                            self.device_manager.pic.lower_irq(irq);
                        }
                    }
                }
                TimerOwner::SerialTx(port_index) => {
                    for _ in 0..counts[entry] {
                        self.device_manager.serial.tx_timer_fired(port_index);
                    }
                    // Re-arm for the next byte if transmission continues
                    // (Bochs serial.cc tx_timer re-activates the timer).
                    if let Some(delay) =
                        self.device_manager.serial.take_tx_timer_update(port_index)
                    {
                        self.devices.request_timer_after_usec(
                            DeviceTimerOwner::SerialTx(port_index),
                            current_ticks,
                            delay,
                        );
                    }
                    for (irq, raise) in self.device_manager.serial.take_pending_irqs() {
                        if raise {
                            self.device_manager.pic.raise_irq(irq);
                        } else {
                            self.device_manager.pic.lower_irq(irq);
                        }
                    }
                }
                TimerOwner::Lapic(cpu_index) => {
                    if cpu_index < self.cpu_count() {
                        self.cpu_mut_at(cpu_index).lapic.timer_fired = true;
                        self.refresh_cpu_masks(cpu_index);
                    }
                }
                #[cfg(feature = "std")]
                TimerOwner::Slowdown => {
                    for _ in 0..counts[entry] {
                        let action = self.slowdown_timer.handle_timer(
                            self.pc_system.time_usec(),
                            std::time::Instant::now(),
                        );
                        if let Some(handle) = self.slowdown_timer.timer_handle {
                            if let Err(error) = self.pc_system.activate_timer_usec(
                                handle,
                                action.next_delay_usec,
                                false,
                            ) {
                                tracing::error!(
                                    "slowdown timer reactivation failed: {error:?}"
                                );
                            }
                        }
                        if action.sleep_one_quantum {
                            std::thread::sleep(std::time::Duration::from_micros(
                                SLOWDOWN_QUANTUM_USEC,
                            ));
                        }
                    }
                }
            }
        }

        let (fwds, forward_count) = self.device_manager.pic.take_ioapic_forwards();
        let DeviceManager {
            ref mut pic,
            ref mut ioapic,
            ..
        } = self.device_manager;
        for &(irq, level) in &fwds[..forward_count] {
            ioapic.set_irq_level(irq, level, Some(&mut *pic), None);
        }
    }

    /// Keep the VGA vertical-retrace timer armed at the current display period.
    ///
    /// Bochs vgacore.cc re-activates `vga_vtimer_id` from
    /// `start_vertical_timer()` whenever `calculate_retrace_timing()` produces a
    /// new `vtotal_usec`. Here the period is polled at the scheduler boundary,
    /// which covers every path that can change the CRTC timing registers.
    pub(super) fn sync_vga_vertical_timer(&mut self) {
        let Some(handle) = self.vga_vertical_timer_handle else {
            return;
        };
        let period = self.device_manager.vga.vertical_period_usec();
        if period == 0 || period == self.vga_vertical_period_usec {
            return;
        }
        match self.pc_system.activate_timer_usec(handle, period, true) {
            Ok(()) => self.vga_vertical_period_usec = period,
            Err(error) => {
                tracing::warn!("failed to arm the VGA vertical timer: {error:?}");
            }
        }
    }

    /// Apply the side effects the HPET queued from MMIO context — the calls
    /// Bochs hpet.cc performs synchronously inside its handlers
    /// (`update_irq`, `activate_timer_nsec`, `deactivate_timer`,
    /// `DEV_pit_enable_irq`, `DEV_cmos_enable_irq`, `DEV_MEM_WRITE_PHYSICAL`).
    /// Comparator deadlines were pre-anchored at the access instant, so the
    /// drain point does not shift them.
    pub(super) fn drain_hpet_pending(&mut self) {
        if !self.device_manager.hpet.has_pending_work() {
            return;
        }
        let pending = self.device_manager.hpet.take_pending();
        if let Some(enabled) = pending.pit_irq_gate {
            self.device_manager.pit.enable_irq(enabled);
        }
        if let Some(enabled) = pending.cmos_irq_gate {
            self.device_manager.cmos.enable_irq(enabled);
        }
        for &(route, level) in &pending.irq_ops[..pending.irq_op_count] {
            if route < 16 {
                // Bochs DEV_pic_raise_irq/DEV_pic_lower_irq: the legacy PIC
                // call also forwards the edge to the IOAPIC pin.
                if level {
                    self.device_manager.pic.raise_irq(route);
                } else {
                    self.device_manager.pic.lower_irq(route);
                }
            } else if route < 24 {
                // GSI 16..23 exist only as IOAPIC pins here. PERMANENTLY
                // RATIFIED Bochs deviation (user decision 2026-07-25) — this
                // is a closed item, not an open one. Bochs update_irq() routes
                // EVERY HPET pin through bx_pic_c::raise_irq(route,
                // BX_IRQ_TYPE_ISA), whose unbounded `(irq_no < 8) ? master :
                // slave` indexing reads slave IRQ_in[route & 7] for route >= 16
                // — an out-of-range access that spuriously asserts ISA IRQ
                // 8..15 (e.g. route 20 -> IRQ12) AND can trip its
                // `BX_PANIC("ISA IRQ %d lost")` host abort. rusty_box declines
                // to reproduce that hardware bug (a phantom ISA edge + possible
                // host crash) and delivers only the architecturally correct
                // IOAPIC pin. Per CLAUDE.md, correctness trumps Bochs
                // literalness for a buggy/non-safe construct.
                let DeviceManager {
                    ref mut pic,
                    ref mut ioapic,
                    ..
                } = self.device_manager;
                ioapic.set_irq_level(route, level, Some(&mut *pic), None);
            } else {
                tracing::error!("HPET: interrupt route {route} beyond IOAPIC pins");
            }
        }
        for &(address, value) in &pending.fsb_writes[..pending.fsb_write_count] {
            // Bochs update_irq FSB path: DEV_MEM_WRITE_PHYSICAL of the
            // 32-bit message. Bochs never advertises the FSB capability bit,
            // so guests do not normally reach this.
            let mut bytes = value.to_le_bytes();
            if let Err(error) = self.memory.write_physical_page(
                &[],
                // DEV_MEM_WRITE_PHYSICAL — a device access, so it must not see
                // SMRAM (Bochs memory.cc `cpu == NULL`).
                crate::memory::CpuMemoryPolicy::device(),
                address,
                bytes.len(),
                &mut bytes,
            ) {
                tracing::error!("HPET: FSB message write to {address:#x} failed: {error:?}");
            }
        }
        for (index, op) in pending.timer_ops.iter().enumerate() {
            let (Some(op), Some(handle)) =
                (op.as_ref(), self.device_manager.hpet.timer_handles[index])
            else {
                continue;
            };
            let result = match op {
                crate::iodev::hpet::HpetTimerOp::ArmAtTicks(deadline) => self
                    .pc_system
                    .activate_timer_at_ticks(handle, *deadline, false),
                crate::iodev::hpet::HpetTimerOp::Deactivate => {
                    self.pc_system.deactivate_timer(handle)
                }
            };
            if let Err(error) = result {
                tracing::error!("HPET: comparator {index} timer update failed: {error:?}");
            }
        }
    }

    /// Apply fixed I/O owner requests after the raw device manager pointer has
    /// been cleared. Phase 2 owns the already registered IDE channels; later
    /// owners retain their table slots until Phase 3 registers their handles.
    pub(crate) fn drain_device_timer_requests(&mut self) {
        let _boundary_requested = self.devices.take_scheduler_boundary_requested();
        let requests = self.devices.take_timer_requests();
        let owners = [
            (
                DeviceTimerOwner::Pit,
                self.device_manager.pit.timer_handle(),
                "PIT",
            ),
            (
                DeviceTimerOwner::Keyboard,
                self.device_manager.keyboard.timer_handle(),
                "keyboard",
            ),
            (
                DeviceTimerOwner::CmosPeriodic,
                self.device_manager.cmos.periodic_timer_handle,
                "CMOS periodic",
            ),
            (
                DeviceTimerOwner::CmosOneSecond,
                self.device_manager.cmos.one_second_timer_handle,
                "CMOS one-second",
            ),
            (
                DeviceTimerOwner::CmosUip,
                self.device_manager.cmos.uip_timer_handle,
                "CMOS UIP",
            ),
            (
                DeviceTimerOwner::AcpiPmOverflow,
                self.device_manager.acpi.overflow_timer_handle,
                "ACPI PM overflow",
            ),
            (
                DeviceTimerOwner::SerialFifo(0),
                self.device_manager.serial.fifo_timer_handle(0),
                "serial FIFO 0",
            ),
            (
                DeviceTimerOwner::SerialFifo(1),
                self.device_manager.serial.fifo_timer_handle(1),
                "serial FIFO 1",
            ),
            (
                DeviceTimerOwner::SerialFifo(2),
                self.device_manager.serial.fifo_timer_handle(2),
                "serial FIFO 2",
            ),
            (
                DeviceTimerOwner::SerialFifo(3),
                self.device_manager.serial.fifo_timer_handle(3),
                "serial FIFO 3",
            ),
            (
                DeviceTimerOwner::SerialTx(0),
                self.device_manager.serial.tx_timer_handle(0),
                "serial TX 0",
            ),
            (
                DeviceTimerOwner::SerialTx(1),
                self.device_manager.serial.tx_timer_handle(1),
                "serial TX 1",
            ),
            (
                DeviceTimerOwner::SerialTx(2),
                self.device_manager.serial.tx_timer_handle(2),
                "serial TX 2",
            ),
            (
                DeviceTimerOwner::SerialTx(3),
                self.device_manager.serial.tx_timer_handle(3),
                "serial TX 3",
            ),
            (
                DeviceTimerOwner::PciIdeCh0,
                self.device_manager.pci_ide.bmdma[0].timer_index,
                "BM-DMA ch0",
            ),
            (
                DeviceTimerOwner::PciIdeCh1,
                self.device_manager.pci_ide.bmdma[1].timer_index,
                "BM-DMA ch1",
            ),
            (
                DeviceTimerOwner::HdSeek(0),
                self.device_manager.harddrv.seek_timer_handles[0][0],
                "HD/CD seek 0-0",
            ),
            (
                DeviceTimerOwner::HdSeek(1),
                self.device_manager.harddrv.seek_timer_handles[0][1],
                "HD/CD seek 0-1",
            ),
            (
                DeviceTimerOwner::HdSeek(2),
                self.device_manager.harddrv.seek_timer_handles[1][0],
                "HD/CD seek 1-0",
            ),
            (
                DeviceTimerOwner::HdSeek(3),
                self.device_manager.harddrv.seek_timer_handles[1][1],
                "HD/CD seek 1-1",
            ),
        ];

        for (owner, handle, label) in owners {
            let Some(handle) = handle else {
                continue;
            };
            match requests.get(owner) {
                TimerRequest::Unchanged => {}
                TimerRequest::Deactivate => {
                    if let Err(error) = self.pc_system.deactivate_timer(handle) {
                        tracing::error!("{label}: timer deactivation failed: {error:?}");
                    }
                }
                TimerRequest::Activate {
                    deadline_ticks,
                    period_ticks,
                    continuous,
                } => {
                    let result = self.pc_system.activate_timer_at_ticks_with_period(
                        handle,
                        deadline_ticks,
                        period_ticks,
                        continuous,
                    );
                    if let Err(error) = result {
                        tracing::error!("{label}: timer activation failed: {error:?}");
                    }
                }
            }
        }
    }
}
