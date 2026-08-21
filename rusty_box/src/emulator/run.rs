#[cfg(feature = "alloc")]
use crate::iodev::vga::VgaDisplayUpdate;
use crate::{
    cpu::{
        cpu::CpuActivityState,
        instrumentation::Instrumentation,
        BxCpuC,
    },
    Result,
};
#[cfg(feature = "alloc")]
use crate::{cpu::CpuError, Error};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::Emulator;

impl<'a, T: Instrumentation> Emulator<T> {
    #[cfg(feature = "alloc")]
    /// Update GUI with VGA text mode changes
    ///
    /// Call this periodically to refresh the display (matching vgacore.cc)
    /// Uses VGA update() function to process text mode and get update data
    pub fn update_gui(&mut self) {
        if let Some(ref mut gui) = self.gui {
            // Bochs vgacore.cc skip_update() calls bx_gui->clear_screen() for a
            // pending sequencer clear-screen request even on frames it skips.
            if self.device_manager.vga.take_pending_clear_screen() {
                gui.clear_screen();
            }
            // Bochs vgacore.cc publishes each completed DAC write to the GUI via
            // palette_change_common(). Drained here because the VGA cannot reach
            // the GUI directly.
            {
                let changes: alloc::vec::Vec<(u8, u8, u8, u8)> =
                    self.device_manager.vga.take_dac_palette_changes().collect();
                for (index, red, green, blue) in changes {
                    let _accepted = gui.palette_change(index, red, green, blue);
                }
            }
            if let Some(update_result) = self.device_manager.vga.update() {
                match update_result {
                    VgaDisplayUpdate::Text(update_result) => {
                        let cursor_x = if update_result.cursor_address < 0x7fff {
                            let offset_from_start = update_result
                                .cursor_address
                                .saturating_sub(update_result.tm_info.start_address);
                            (offset_from_start % update_result.tm_info.line_offset) / 2
                        } else {
                            0xffff
                        };

                        let cursor_y = if update_result.cursor_address < 0x7fff {
                            let offset_from_start = update_result
                                .cursor_address
                                .saturating_sub(update_result.tm_info.start_address);
                            (offset_from_start / update_result.tm_info.line_offset) as u32
                        } else {
                            0xffff
                        };

                        if update_result.dimension_changed {
                            gui.dimension_update(
                                update_result.iwidth,
                                update_result.iheight,
                                update_result.fheight,
                                update_result.fwidth,
                                8,
                            );
                        }

                        // Bochs vgacore.cc update_charmap() pushes both guest
                        // character generators to the GUI (set_text_charmap)
                        // before the text is drawn with them.
                        if update_result.charmap_updated {
                            gui.set_text_charmap(0, self.device_manager.vga.charmap(0));
                            gui.set_text_charmap(1, self.device_manager.vga.charmap(1));
                        }

                        gui.text_update(
                            &update_result.text_snapshot,
                            &update_result.text_buffer,
                            cursor_x as u32,
                            cursor_y,
                            &update_result.tm_info,
                        );
                    }
                    VgaDisplayUpdate::Graphics(update_result) => {
                        if update_result.dimension_changed {
                            gui.dimension_update(
                                update_result.width,
                                update_result.height,
                                0,
                                0,
                                update_result.bpp as u32,
                            );
                        }
                        for tile in update_result.tiles {
                            gui.graphics_tile_update_rgba(
                                &tile.rgba,
                                tile.x,
                                tile.y,
                                tile.width,
                                tile.height,
                            );
                        }
                    }
                }
            }

            gui.flush();
        }
    }

    /// Drain pending host input (keyboard scancodes, mouse, serial) from the GUI
    /// into the device layer.
    ///
    /// Called from the active step loop AND from inside the HLT/MWAIT idle waits.
    /// The idle path is the important one: a tickless (NO_HZ) guest raises no
    /// periodic timer while halted, so without pumping here a keypress would sit
    /// in the GUI queue until the halt budget expires (~seconds), making input
    /// feel laggy and drop characters. Pumping inside the wait lets a keypress
    /// enqueue a scancode, which the very next device tick delivers as IRQ1,
    /// waking the guest within a device quantum.
    /// Without alloc there is no GUI (`Emulator::gui` requires `Box<dyn
    /// BxGui>`), so host-input pumping is a no-op; `step_batch` and the
    /// HLT/MWAIT waits stay callable from no-alloc hosts like the UEFI
    /// example.
    #[cfg(not(feature = "alloc"))]
    #[inline]
    pub(super) fn pump_gui_input(&mut self) {}

    #[cfg(feature = "alloc")]
    pub(super) fn pump_gui_input(&mut self) {
        let mut scancodes_to_send = Vec::new();
        let mut mouse_to_send = Vec::new();
        let mut serial_input = Vec::new();
        let mut keys_to_send: Vec<(crate::iodev::scancodes::BxKey, bool)> = Vec::new();
        if let Some(gui) = &mut self.gui {
            gui.handle_events();
            scancodes_to_send = gui.get_pending_scancodes();
            keys_to_send = gui.get_pending_keys();
            mouse_to_send = gui.get_pending_mouse();
            serial_input = gui.get_pending_serial_input();
        }
        let keyboard_changed = !scancodes_to_send.is_empty()
            || !keys_to_send.is_empty()
            || !mouse_to_send.is_empty();
        let serial_changed = !serial_input.is_empty();
        for (key, pressed) in keys_to_send {
            self.device_manager.keyboard.gen_scancode(key, pressed);
        }
        for scancode in scancodes_to_send {
            self.device_manager.keyboard.send_scancode(scancode);
        }
        for mouse in mouse_to_send {
            self.device_manager
                .keyboard
                .mouse_motion(mouse.dx, mouse.dy, mouse.dz, mouse.buttons);
        }
        for byte in serial_input {
            self.device_manager.serial.receive_byte(0, byte);
        }
        if !keyboard_changed && !serial_changed {
            return;
        }

        let current_ticks = self.pc_system.time_ticks();
        // Keyboard input needs no timer arming: the continuous 8042
        // serial-delay timer (Bochs keyboard.cc) picks queued bytes up on
        // its next fire.
        if serial_changed {
            // A host byte arriving at the UART leaves the same latched
            // interrupt and FIFO-timeout work a guest-visible access would, so
            // it is drained through the device API rather than by hand.
            self.drain_serial_effects(0, current_ticks);
        }
        // A reset applied here needs no branch: host input reaches a machine
        // that resumes at the reset vector either way.
        if let Err(error) = self.service_scheduler_boundary(0) {
            tracing::error!("host-input scheduler boundary failed: {error:?}");
        }
    }

    #[cfg(feature = "alloc")]
    /// Set up direct Linux kernel boot, bypassing BIOS entirely.
    ///
    /// Loads a bzImage kernel and optional initramfs into memory, sets up
    /// the Linux boot protocol "zero page" (boot_params), configures CPU
    /// for 32-bit protected mode, and points EIP at the kernel entry.
    ///
    /// This is equivalent to QEMU's `-kernel` / `-initrd` / `-append` options.
    ///
    /// # Arguments
    /// * `bzimage` - Raw bzImage kernel file contents
    /// * `initramfs` - Optional initramfs/initrd file contents
    /// * `cmdline` - Kernel command line string
    ///
    /// # Memory Layout
    /// * 0x1000: GDT (4 entries)
    /// * 0x10000: boot_params (4096 bytes)
    /// * 0x20000: command line (up to 2048 bytes)
    /// * 0x100000: protected-mode kernel
    /// * High memory: initramfs (if provided)
    pub fn setup_direct_linux_boot(
        &mut self,
        bzimage: &[u8],
        initramfs: Option<&[u8]>,
        cmdline: &str,
    ) -> Result<()> {
        let ram_size = self.config.guest_memory_size as u64;
        let cpu_count = self.config.cpu_params.cpu_count();

        // Shared implementation in `crate::boot` — one boot path serves both
        // the alloc Emulator and the raw (cpu, memory) no-alloc entry point.
        // The emulator's complete stable pin set protects every CPU's cached
        // TLB windows while guest RAM is written.
        crate::boot::setup_direct_linux_boot_with_pins(
            &mut *self.cpu,
            &mut self.memory,
            bzimage,
            initramfs,
            cmdline.as_bytes(),
            ram_size,
            cpu_count,
        )
        .map_err(|err| {
            Error::Cpu(CpuError::InvalidBootImage {
                reason: match err {
                    crate::boot::BootError::BzImageTooSmall => "bzImage too small",
                    crate::boot::BootError::InvalidBootSignature => {
                        "Invalid bzImage boot signature"
                    }
                    crate::boot::BootError::InvalidHeaderMagic => "Invalid bzImage header magic",
                    crate::boot::BootError::BootProtocolTooOld => {
                        "boot protocol too old (need >= 2.04)"
                    }
                    crate::boot::BootError::MemoryLoadFailed => {
                        "failed to load boot image into guest RAM"
                    }
                    crate::boot::BootError::InvalidCpuCount => "invalid CPU count for direct boot",
                },
            })
        })?;

        // =====================================================================
        // Initialize PIC (normally done by BIOS POST)
        // Direct boot skips BIOS, so we must set up the interrupt controllers
        // manually. The kernel needs timer interrupts (IRQ0) for calibration
        // and early init functions that call udelay()/mdelay().
        // =====================================================================
        {
            // Initialize master PIC: ICW1-ICW4
            // ICW1: edge-triggered, cascade, ICW4 needed
            self.device_manager.pic.write(0x20, 0x11, 1);
            // ICW2: master vectors 0x20-0x27 (Linux kernel expects IRQ0=0x20)
            self.device_manager.pic.write(0x21, 0x20, 1);
            // ICW3: slave on IRQ2
            self.device_manager.pic.write(0x21, 0x04, 1);
            // ICW4: 8086 mode, normal EOI
            self.device_manager.pic.write(0x21, 0x01, 1);
            // OCW1: mask all master IRQs — kernel will unmask what it needs
            self.device_manager.pic.write(0x21, 0xFF, 1);

            // Initialize slave PIC: ICW1-ICW4
            self.device_manager.pic.write(0xA0, 0x11, 1);
            // ICW2: slave vectors 0x28-0x2F (Linux kernel expects IRQ8=0x28)
            self.device_manager.pic.write(0xA1, 0x28, 1);
            // ICW3: cascade identity = 2
            self.device_manager.pic.write(0xA1, 0x02, 1);
            // ICW4: 8086 mode
            self.device_manager.pic.write(0xA1, 0x01, 1);
            // OCW1: mask all slave IRQs
            self.device_manager.pic.write(0xA1, 0xFF, 1);

            // Do NOT program PIT — kernel will set up its own timer via time_init().
            // quick_pit_calibrate() programs PIT C2 via port 0x43/0x42 directly.
            tracing::debug!(
                "Direct boot: PIC initialized (master=0x20, slave=0x28), all IRQs masked"
            );
        }

        Ok(())
    }

    /// Execute a batch of instructions cooperatively (no blocking loop).
    ///
    /// Designed for single-threaded environments like WASM or UEFI where the
    /// caller must yield control back to its event loop regularly. Runs up to
    /// `max_instructions`, ticks devices, syncs A20, then returns.
    ///
    /// Returns `(instructions_executed, is_shutdown)`.
    pub fn step_batch(&mut self, max_instructions: u64) -> Result<(u64, bool)> {
        let ips = self.config.ips as u64;
        let mut total_executed = 0u64;
        // Wall-clock budget: 15ms keeps GUI responsive at 60 fps.
        // Bochs runs CPU on a dedicated thread with no frame budget; we emulate
        // that throughput by processing multiple MWAIT→wake→execute cycles here.
        #[cfg(feature = "std")]
        let wall_start = std::time::Instant::now();
        #[cfg(feature = "std")]
        let wall_budget = std::time::Duration::from_millis(15);

        'batch: loop {
            // --- Run CPU batch ---
            // SAFETY: see borrow_memory_for_cpu / run_cpu_batch
            let result = self.run_cpu_batch(max_instructions);

            let executed = match result {
                Ok(n) => n,
                Err(e) => return Err(crate::error::Error::Cpu(e)),
            };
            total_executed += executed;

            // --- Tick devices + pc_system ---
            if !self.batch_advanced_pc_system {
                self.advance_pc_system_after_cpu_ticks(executed);
            }

            // --- HLT/MWAIT: advance time until interrupt ---
            if matches!(
                self.cpu.activity_state,
                CpuActivityState::Hlt | CpuActivityState::Mwait | CpuActivityState::MwaitIf
            ) && self.can_fast_forward_bsp_hlt()
            {
                let mwait_if = matches!(self.cpu.activity_state, CpuActivityState::MwaitIf);
                let mut hlt_budget = 0u64;
                while hlt_budget < 100_000_000 {
                    // Service host input while halted (see run_interactive).
                    self.pump_gui_input();
                    if self.has_interrupt() && (self.cpu.interrupts_enabled() || mwait_if) {
                        break;
                    }
                    self.service_lapic_local_events();
                    if self.cpu.lapic.intr && (self.cpu.interrupts_enabled() || mwait_if) {
                        self.cpu
                            .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                        break;
                    }
                    let step = self.hlt_wait_step_ticks();
                    if self.service_scheduler_boundary(u64::from(step))? {
                        // Reset: stop advancing time; the CPU is Active at
                        // the reset vector.
                        break;
                    }
                    hlt_budget += u64::from(step);
                    if !self.can_fast_forward_bsp_hlt() {
                        break;
                    }
                }
                if self.cpu.lapic_has_intr() {
                    self.cpu
                        .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                }
            }

            // --- Deliver PIC interrupt ---
            if self.device_manager.has_interrupt()
                && self.cpu.get_b_if() != 0
                && !self.cpu.interrupts_inhibited(0x01)
                // Bochs event.cc delivers Priority-4 debug traps before
                // Priority-5 external interrupts; defer one boundary so the
                // CPU can deliver #DB first. The interrupt stays latched in
                // the PIC because iac() is not called.
                && !self.cpu.debug_trap_pending()
            {
                let vector = self.iac();
                // SAFETY: see borrow_memory_for_cpu / inject_interrupt
                if let Err(e) = self.inject_interrupt(vector) {
                    tracing::warn!("PIC interrupt injection (vector {vector:#04x}) failed: {e:?}");
                }
            }

            // --- Tight loop: if CPU was woken from MWAIT and wall budget remains,
            // run another cycle instead of returning to egui event loop.
            // This matches Bochs's dedicated CPU thread which never yields to GUI.
            // Without std there is no wall clock: yield cooperatively on the
            // instruction budget instead, so an Active CPU cannot spin here
            // forever and starve the caller's event loop.
            if matches!(self.cpu.activity_state, CpuActivityState::Active) && {
                #[cfg(feature = "std")]
                {
                    wall_start.elapsed() < wall_budget
                }
                #[cfg(not(feature = "std"))]
                {
                    total_executed < max_instructions
                }
            } {
                continue 'batch;
            }

            break 'batch;
        }


        // Handle keyboard/mouse/serial input from GUI.
        self.pump_gui_input();

        let shutdown = self.cpu.is_in_shutdown();
        Ok((total_executed, shutdown))
    }

    #[cfg(feature = "alloc")]
    /// Render VGA text output into a `SharedDisplay` framebuffer.
    ///
    /// This is the single-threaded equivalent of `update_gui()` — instead of
    /// going through the `BxGui` trait (which requires `Arc<Mutex<>>` for
    /// thread-safe sharing), it writes directly to the provided display.
    /// Ideal for WASM where the emulator and display are owned by the same
    /// event loop.
    pub fn update_display(&mut self, display: &mut crate::gui::shared_display::SharedDisplay) {
        #[cfg(debug_assertions)]
        let dbg = {
            static DBG_CTR: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            DBG_CTR.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
        };

        if let Some(update_result) = self.device_manager.vga.update() {
            match update_result {
                VgaDisplayUpdate::Text(update_result) => {
                    #[cfg(debug_assertions)]
                    if dbg % 300 == 1 {
                        let non_zero = update_result
                            .text_buffer
                            .iter()
                            .filter(|&&b| b != 0)
                            .count();
                        let first_16: Vec<u8> =
                            update_result.text_buffer.iter().take(32).copied().collect();
                        tracing::trace!(
                            "VGA update: dim_changed={}, needs_update={}, buf_non_zero={}, first_32={:02x?}, start_addr={}",
                            update_result.dimension_changed,
                            update_result.needs_update,
                            non_zero,
                            first_16,
                            update_result.tm_info.start_address,
                        );
                    }
                    let cursor_x = if update_result.cursor_address < 0x7fff {
                        let offset_from_start = update_result
                            .cursor_address
                            .saturating_sub(update_result.tm_info.start_address);
                        (offset_from_start % update_result.tm_info.line_offset) / 2
                    } else {
                        0xffff
                    };

                    let cursor_y = if update_result.cursor_address < 0x7fff {
                        let offset_from_start = update_result
                            .cursor_address
                            .saturating_sub(update_result.tm_info.start_address);
                        (offset_from_start / update_result.tm_info.line_offset) as u32
                    } else {
                        0xffff
                    };

                    if update_result.dimension_changed {
                        display.resize(
                            update_result
                                .iwidth
                                .checked_div(update_result.fwidth)
                                .unwrap_or(update_result.iwidth),
                            update_result
                                .iheight
                                .checked_div(update_result.fheight)
                                .unwrap_or(update_result.iheight),
                            update_result.fwidth,
                            update_result.fheight,
                        );
                    }

                    display.render_text_to_framebuffer(
                        &update_result.text_buffer,
                        cursor_x as u32,
                        cursor_y,
                        update_result.tm_info.cs_start,
                        update_result.tm_info.cs_end,
                        update_result.tm_info.line_graphics,
                        update_result.tm_info.start_address as u32,
                        update_result.tm_info.line_offset as u32,
                        &update_result.tm_info.actl_palette,
                    );
                }
                VgaDisplayUpdate::Graphics(update_result) => {
                    if update_result.dimension_changed {
                        display.resize_pixels(update_result.width, update_result.height);
                    }
                    for tile in update_result.tiles {
                        display.blit_rgba_tile(tile.x, tile.y, tile.width, tile.height, &tile.rgba);
                    }
                }
            }
        }
    }

    /// Send a PS/2 scancode to the keyboard device.
    ///
    /// For environments that handle keyboard input outside of `BxGui`
    /// (e.g. the WASM app processes egui events directly).
    /// Deliver a guest key press/release, rendered through the guest's active
    /// scancode set (Bochs keyboard.cc `gen_scancode`). Prefer this over
    /// [`Emulator::send_scancode`], which bypasses the set selection.
    pub fn send_key(&mut self, key: crate::iodev::scancodes::BxKey, pressed: bool) {
        self.device_manager.keyboard.gen_scancode(key, pressed);
    }

    pub fn send_scancode(&mut self, scancode: u8) {
        self.device_manager.keyboard.send_scancode(scancode);
    }

    /// Send a relative PS/2 mouse update to the aux (mouse) device.
    ///
    /// Deltas are in mouse counts; `buttons` is a bitmask (bit 0 = left,
    /// bit 1 = right, bit 2 = middle). Mirrors [`send_scancode`] for the WASM
    /// path and the native input pump. Bochs keyboard.cc mouse_motion.
    pub fn send_mouse_event(&mut self, dx: i32, dy: i32, dz: i32, buttons: u8) {
        self.device_manager
            .keyboard
            .mouse_motion(dx, dy, dz, buttons);
    }

    #[cfg(feature = "alloc")]
    /// Send a string as PS/2 Set 2 scancodes (make + break for each character).
    ///
    /// Useful for headless testing — inject "root\n" to type at a login prompt.
    /// Each character is converted to its scancode sequence including shift
    /// modifier when needed.
    pub fn send_string(&mut self, text: &str) {
        for ch in text.chars() {
            let scancodes = crate::gui::keymap::char_to_scancode_sequence(ch);
            for &sc in &scancodes {
                self.device_manager.keyboard.send_scancode(sc);
            }
        }
    }

    /// Force VGA to generate an initial update (call before first `update_display`).
    pub fn force_vga_update(&mut self) {
        self.device_manager.vga.force_initial_update();
    }

    /// Initialize VGA to standard text mode 3 (80x25 color).
    /// Must be called for direct kernel boot where no BIOS runs.
    pub fn init_vga_text_mode3(&mut self) {
        self.device_manager.vga.init_text_mode3();
    }
}
