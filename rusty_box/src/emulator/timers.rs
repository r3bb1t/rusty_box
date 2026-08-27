use crate::{
    cpu::{
        instrumentation::Instrumentation,
    },
    iodev::{DeviceTimerOwner, TimerRequest},
    pc_system::TimerOwner, Result,
};

use super::Emulator;
#[cfg(feature = "std")]
use super::SLOWDOWN_QUANTUM_USEC;

impl<'a, T: Instrumentation> Emulator<T> {
    /// Give every device that needs one a slot in the timer wheel, and hand
    /// each device its handle.
    ///
    /// Bochs registers these from the individual device `init` methods
    /// (pit.cc, keyboard.cc, …); here the wheel is machine-owned, so
    /// registration happens once, from device initialisation, and a device
    /// keeps only its handle.
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
            self.device_manager.ide.bus_master.bmdma[channel].timer_index = Some(handle);
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
                self.device_manager.ide.drives.seek_timer_handles[channel][device] = Some(handle);
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
        // The UART owns its scheduler slots directly, so its timers are
        // disarmed on the wheel rather than through the request table.
        for port_index in 0..self.device_manager.serial.configured_port_count() {
            for handle in [
                self.device_manager.serial.fifo_timer_handle(port_index),
                self.device_manager.serial.tx_timer_handle(port_index),
            ]
            .into_iter()
            .flatten()
            {
                if let Err(error) = self.pc_system.deactivate_timer(handle) {
                    tracing::error!("serial timer {handle} failed to disarm on reset: {error:?}");
                }
            }
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
        let ips = self.config.ips.per_second_u64();
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
                    self.device_manager
                        .vga
                        .vertical_timer(now_usec, current_ticks);
                }
                TimerOwner::PciIdeCh0 => {
                    for _ in 0..counts[entry] {
                        self.device_manager
                            .pci_ide_timer(0, &mut self.pc_system, &mut self.memory);
                    }
                }
                TimerOwner::PciIdeCh1 => {
                    for _ in 0..counts[entry] {
                        self.device_manager
                            .pci_ide_timer(1, &mut self.pc_system, &mut self.memory);
                    }
                }
                TimerOwner::HdSeek(param) => {
                    // Bochs harddrv.cc seek_timer — one-shot; the seek deadline
                    // completes the read command (DRQ/IRQ or BM-DMA start).
                    for _ in 0..counts[entry] {
                        let crate::iodev::devices::DeviceManager { ide, irq, .. } =
                            &mut self.device_manager;
                        ide.seek_timer(param as u8, irq);
                    }
                }
                // The PIT replays its own OUT transitions onto IRQ0 and re-arms
                // itself from inside the callback (Bochs pit.cc handle_timer).
                TimerOwner::Pit => {
                    for _ in 0..counts[entry] {
                        self.fire_pit_timer(current_ticks);
                    }
                }
                TimerOwner::Hpet(index) => {
                    // Bochs hpet.cc timer_handler → hpet_timer(): runs in
                    // emulator context, so the queued IRQ edges and the
                    // comparator re-arm drain immediately.
                    for _ in 0..counts[entry] {
                        let clock = self.pc_system.clock_at(current_ticks);
                        self.device_manager.hpet.set_now(clock);
                        self.device_manager.hpet.timer_fired(index);
                        self.drain_hpet_pending();
                    }
                }
                TimerOwner::Keyboard => {
                    // Bochs keyboard.cc timer_handler: the continuous
                    // serial-delay timer runs periodic(1) every fire and
                    // raises whatever IRQs the controller latched since the
                    // previous fire. No rearm — pc_system reloads the period.
                    self.fire_keyboard_timer(counts[entry], current_ticks);
                }
                // The RTC raises IRQ8 and arms its own UIP pulse from inside
                // the callback, as Bochs cmos.cc does.
                TimerOwner::CmosPeriodic => self.fire_cmos_timer(
                    crate::iodev::cmos::BxCmosC::PERIODIC_TIMER_LOCAL,
                    counts[entry],
                    current_ticks,
                ),
                TimerOwner::CmosOneSecond => self.fire_cmos_timer(
                    crate::iodev::cmos::BxCmosC::ONE_SECOND_TIMER_LOCAL,
                    counts[entry],
                    current_ticks,
                ),
                TimerOwner::CmosUip => self.fire_cmos_timer(
                    crate::iodev::cmos::BxCmosC::UIP_TIMER_LOCAL,
                    counts[entry],
                    current_ticks,
                ),
                TimerOwner::AcpiPmOverflow => {
                    let mut handles = crate::iodev::wiring::TimerHandles::default();
                    handles.set(
                        crate::iodev::acpi::BxAcpiCtrl::OVERFLOW_TIMER_LOCAL,
                        self.device_manager.acpi.overflow_timer_handle,
                    );
                    let crate::iodev::devices::DeviceManager {
                        ref mut acpi,
                        ref mut irq,
                        ..
                    } = self.device_manager;
                    crate::iodev::wiring::with_device_ctx(
                        irq,
                        &mut self.pc_system,
                        handles,
                        current_ticks,
                        |ctx| {
                            rusty_box_devices::api::TimedDevice::timer_fired(
                                acpi,
                                crate::iodev::acpi::BxAcpiCtrl::OVERFLOW_TIMER_LOCAL,
                                counts[entry],
                                ctx,
                            )
                        },
                    );
                }
                // Both UART timers run through the device API: the device
                // raises its own interrupts and re-arms itself from inside the
                // callback, as Bochs serial.cc tx_timer does.
                TimerOwner::SerialFifo(port_index) => {
                    self.fire_serial_timer(
                        port_index,
                        crate::iodev::serial::BxSerialC::fifo_timer_local(port_index),
                        counts[entry],
                        current_ticks,
                    );
                }
                TimerOwner::SerialTx(port_index) => {
                    self.fire_serial_timer(
                        port_index,
                        crate::iodev::serial::BxSerialC::tx_timer_local(port_index),
                        counts[entry],
                        current_ticks,
                    );
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
    /// Run `action` against the UART with the same capabilities it receives
    /// during port I/O, so interrupts it raises and timers it arms take effect
    /// immediately instead of being latched for a later boundary.
    pub(super) fn with_serial_ctx<R>(
        &mut self,
        port_index: usize,
        current_ticks: u64,
        action: impl FnOnce(
            &mut crate::iodev::serial::BxSerialC,
            &mut rusty_box_devices::api::DeviceCtx<'_>,
        ) -> R,
    ) -> R {
        let mut handles = crate::iodev::wiring::TimerHandles::default();
        handles.set(
            crate::iodev::serial::BxSerialC::fifo_timer_local(port_index),
            self.device_manager.serial.fifo_timer_handle(port_index),
        );
        handles.set(
            crate::iodev::serial::BxSerialC::tx_timer_local(port_index),
            self.device_manager.serial.tx_timer_handle(port_index),
        );

        let crate::iodev::devices::DeviceManager {
            ref mut serial,
            ref mut irq,
            ..
        } = self.device_manager;
        crate::iodev::wiring::with_device_ctx(
            irq,
            &mut self.pc_system,
            handles,
            current_ticks,
            |ctx| action(serial, ctx),
        )
    }

    /// Service the 8042's continuous serial-delay timer through the device API.
    fn fire_keyboard_timer(&mut self, fires: u32, current_ticks: u64) {
        let crate::iodev::devices::DeviceManager {
            ref mut keyboard,
            ref mut irq,
            ..
        } = self.device_manager;
        crate::iodev::wiring::with_device_ctx(
            irq,
            &mut self.pc_system,
            crate::iodev::wiring::TimerHandles::default(),
            current_ticks,
            |ctx| rusty_box_devices::api::TimedDevice::timer_fired(keyboard, 0, fires, ctx),
        );
    }

    /// Service one expiry of the PIT's event timer through the device API.
    fn fire_pit_timer(&mut self, current_ticks: u64) {
        let mut handles = crate::iodev::wiring::TimerHandles::default();
        handles.set(
            crate::iodev::pit::BxPitC::EVENT_TIMER_LOCAL,
            self.device_manager.pit.timer_handle,
        );
        let crate::iodev::devices::DeviceManager {
            ref mut pit,
            ref mut irq,
            ..
        } = self.device_manager;
        crate::iodev::wiring::with_device_ctx(
            irq,
            &mut self.pc_system,
            handles,
            current_ticks,
            |ctx| {
                rusty_box_devices::api::TimedDevice::timer_fired(
                    pit,
                    crate::iodev::pit::BxPitC::EVENT_TIMER_LOCAL,
                    1,
                    ctx,
                )
            },
        );
    }

    /// Service one expiry of an RTC timer through the device API.
    fn fire_cmos_timer(&mut self, local: u16, fires: u32, current_ticks: u64) {
        let mut handles = crate::iodev::wiring::TimerHandles::default();
        handles.set(
            crate::iodev::cmos::BxCmosC::PERIODIC_TIMER_LOCAL,
            self.device_manager.cmos.periodic_timer_handle,
        );
        handles.set(
            crate::iodev::cmos::BxCmosC::ONE_SECOND_TIMER_LOCAL,
            self.device_manager.cmos.one_second_timer_handle,
        );
        handles.set(
            crate::iodev::cmos::BxCmosC::UIP_TIMER_LOCAL,
            self.device_manager.cmos.uip_timer_handle,
        );
        let crate::iodev::devices::DeviceManager {
            ref mut cmos,
            ref mut irq,
            ..
        } = self.device_manager;
        crate::iodev::wiring::with_device_ctx(
            irq,
            &mut self.pc_system,
            handles,
            current_ticks,
            |ctx| rusty_box_devices::api::TimedDevice::timer_fired(cmos, local, fires, ctx),
        );
    }

    /// Service one expiry of a UART timer.
    fn fire_serial_timer(&mut self, port_index: usize, local: u16, fires: u32, current_ticks: u64) {
        self.with_serial_ctx(port_index, current_ticks, |serial, ctx| {
            rusty_box_devices::api::TimedDevice::timer_fired(serial, local, fires, ctx)
        });
    }

    /// Deliver interrupt and timer work the UART latched outside guest I/O —
    /// a host byte pushed into the receive path leaves the same state a
    /// guest-visible access would. Only a host frontend delivers such bytes,
    /// so this follows `pump_gui_input` behind the `alloc` gate.
    #[cfg(feature = "alloc")]
    pub(super) fn drain_serial_effects(&mut self, port_index: usize, current_ticks: u64) {
        self.with_serial_ctx(port_index, current_ticks, |serial, ctx| {
            serial.drain_pending_effects(ctx, port_index)
        });
    }

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
                self.device_manager
                    .irq
                    .set_isa_level(rusty_box_devices::api::IrqLine(route), level);
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
                self.device_manager.irq.set_ioapic_pin(route, level);
            } else {
                tracing::error!("HPET: interrupt route {route} beyond IOAPIC pins");
            }
        }
        for &(address, value) in &pending.fsb_writes[..pending.fsb_write_count] {
            // Bochs update_irq FSB path: DEV_MEM_WRITE_PHYSICAL of the
            // 32-bit message. Bochs never advertises the FSB capability bit,
            // so guests do not normally reach this.
            let mut bytes = value.to_le_bytes();
            match self.memory.write_physical_page(// DEV_MEM_WRITE_PHYSICAL — a device access, so it must not see
                // SMRAM (Bochs memory.cc `cpu == NULL`).
                crate::memory::CpuMemoryPolicy::device(),
                address,
                bytes.len(),
                &mut bytes,
            ) {
                Ok(crate::memory::PhysAccess::Done) => {}
                // A device writing into another device's window. Bochs would
                // recurse into the target's handler from inside the memory
                // write; the routing is explicit here instead.
                Ok(crate::memory::PhysAccess::Mmio(hit)) => {
                    // Through the same choke point a guest access takes, so
                    // the two cannot disagree about how a token is routed.
                    // Time is the wheel's, because this runs at the boundary
                    // rather than inside a batch.
                    let now_ticks = self.pc_system.time_ticks();
                    if !self.devices.mmio_write(
                        hit,
                        bytes.len() as u32,
                        &bytes,
                        now_ticks,
                        &mut self.pc_system,
                        &mut self.device_manager,
                    ) {
                        tracing::error!(
                            "HPET: FSB message to {address:#x} routed to a slot with no device"
                        );
                    }
                }
                Err(error) => {
                    tracing::error!("HPET: FSB message write to {address:#x} failed: {error:?}");
                }
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
        // I/O raises the boundary request to say it queued timer work, and this
        // is the boundary that services it — so the request is answered here,
        // not dropped. Nothing can raise it between the two halves: a device
        // timer callback reaches `request_timer`, never the port bus.
        let requests = self.devices.take_boundary_timer_requests();
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
                DeviceTimerOwner::PciIdeCh0,
                self.device_manager.ide.bus_master.bmdma[0].timer_index,
                "BM-DMA ch0",
            ),
            (
                DeviceTimerOwner::PciIdeCh1,
                self.device_manager.ide.bus_master.bmdma[1].timer_index,
                "BM-DMA ch1",
            ),
            (
                DeviceTimerOwner::HdSeek(0),
                self.device_manager.ide.drives.seek_timer_handles[0][0],
                "HD/CD seek 0-0",
            ),
            (
                DeviceTimerOwner::HdSeek(1),
                self.device_manager.ide.drives.seek_timer_handles[0][1],
                "HD/CD seek 0-1",
            ),
            (
                DeviceTimerOwner::HdSeek(2),
                self.device_manager.ide.drives.seek_timer_handles[1][0],
                "HD/CD seek 1-0",
            ),
            (
                DeviceTimerOwner::HdSeek(3),
                self.device_manager.ide.drives.seek_timer_handles[1][1],
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
