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

use super::{Emulator, ResetReason};
// Only the direct-Linux-boot path below reaches a CPU through the store, and
// that path needs an allocator.
#[cfg(feature = "alloc")]
use super::cpu_store::CpuStore;

/// What state a machine's power is in.
///
/// Asked at any time, unlike [`StopReason`], which explains why one particular
/// batch returned. A caller that has just restored a snapshot, or that has not
/// stepped yet, has no batch outcome to read.
///
/// Closed on purpose (R5): a machine that learns a new power state should
/// break every caller that decides what to do about it, rather than have them
/// fall into a wildcard. `#[non_exhaustive]` would buy semver room this
/// pre-1.0 surface has not asked for and cost exactly that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerState {
    /// Executing, or halted waiting for an interrupt — either way, alive.
    Running,
    /// The guest asked to be powered off: ACPI `PM1_CNT` with `SLP_EN` and
    /// `SLP_TYP` = S5, or the port-0x8900 shutdown protocol. The CPU is
    /// perfectly healthy; it is the machine that is finished.
    PoweredOff,
    /// The boot CPU is in the architectural shutdown state, a triple fault
    /// being the usual way in.
    CpuShutdown,
}

/// The machine's power and reset controls — Bochs's chipset-level ACPI, which
/// every profile has, so this is not a device that can be absent.
pub struct Power<'m, T: Instrumentation> {
    machine: &'m mut Emulator<T>,
}

impl<T: Instrumentation> Power<'_, T> {
    /// Press the power button.
    ///
    /// This is a request to the guest, not an order: it raises ACPI's
    /// `PWRBTN_STS` and re-evaluates SCI, exactly as Bochs acpi.cc does. An
    /// ACPI-aware guest takes the interrupt and shuts itself down, at which
    /// point a batch reports [`StopReason::GuestPowerOff`]. A guest that never
    /// enabled `PWRBTN_EN`, or that chooses to ignore it, keeps running — as
    /// it would on real hardware.
    pub fn press_power_button(&mut self) {
        let ticks = self.machine.pc_system.time_ticks();
        self.machine
            .device_manager
            .acpi
            .press_power_button(ticks);
    }

    /// Reset the machine. Unlike the power button this is not negotiable with
    /// the guest.
    pub fn reset(&mut self, reason: ResetReason) -> Result<()> {
        self.machine.reset(reason)
    }

    /// The machine's current power state.
    pub fn state(&self) -> PowerState {
        if self.machine.cpu_ref(0).is_in_shutdown() {
            return PowerState::CpuShutdown;
        }
        if self
            .machine
            .stop_flag
            .load(core::sync::atomic::Ordering::Relaxed)
            && self.machine.stop_cause == StopCause::GuestPowerOff
        {
            return PowerState::PoweredOff;
        }
        PowerState::Running
    }
}

/// The machine's keyboard, as a host driving it sees it.
///
/// Every verb reports how much got through. The i8042's ring is 16 bytes and a
/// guest that is not draining it fills it quickly, so a host typing into a busy
/// machine WILL overrun — Bochs and QEMU both drop silently at that point, and
/// the caller finds out by the guest missing keystrokes. Reporting instead
/// turns that into backpressure: send, see how much landed, step the machine,
/// send the rest.
pub struct Keyboard<'m, T: Instrumentation> {
    machine: &'m mut Emulator<T>,
}

impl<T: Instrumentation> Keyboard<'_, T> {
    /// Press or release a key, rendered through the guest's active scancode
    /// set. Returns whether the whole sequence reached the guest — see
    /// `BxKeyboardC::gen_scancode` for why a key can be partly delivered.
    #[must_use = "a refused key never reached the guest"]
    pub fn key(&mut self, key: crate::iodev::scancodes::BxKey, pressed: bool) -> bool {
        self.machine
            .device_manager
            .keyboard
            .gen_scancode(key, pressed)
    }

    /// Press and release a key. `false` if either half was not delivered
    /// intact — including the case where the press landed and the release did
    /// not, which a guest sees as a stuck key.
    #[must_use = "a half-delivered tap leaves the guest holding the key down"]
    pub fn tap(&mut self, key: crate::iodev::scancodes::BxKey) -> bool {
        let down = self.key(key, true);
        let up = self.key(key, false);
        down && up
    }

    /// Feed raw scancode bytes, returning how many were accepted.
    ///
    /// Stops at the first byte the ring refuses, so the count is also the
    /// resume point: `bytes[accepted..]` is exactly what still has to be sent.
    #[must_use = "a short count means the guest did not receive the rest"]
    pub fn scancodes(&mut self, bytes: &[u8]) -> usize {
        let mut accepted = 0;
        for &byte in bytes {
            if !self.machine.device_manager.keyboard.send_scancode(byte) {
                break;
            }
            accepted += 1;
        }
        accepted
    }

    /// Type text, returning how many CHARACTERS were delivered whole.
    ///
    /// Characters, not bytes: one character can be several scancodes, and a
    /// count of bytes would let a caller resume mid-key and hand the guest a
    /// prefix with no code. Resume from `text[..].chars().skip(n)`.
    #[cfg(feature = "alloc")]
    #[must_use = "a short count means the guest did not receive the rest of the text"]
    pub fn type_text(&mut self, text: &str) -> usize {
        let mut typed = 0;
        for ch in text.chars() {
            let scancodes = crate::gui::keymap::char_to_scancode_sequence(ch);
            // A character is delivered or it is not; a partly-sent one is the
            // corruption this count exists to prevent, so stop at the first
            // refusal rather than pushing the remaining bytes in after it.
            if self.scancodes(&scancodes) != scancodes.len() {
                break;
            }
            typed += 1;
        }
        typed
    }
}

/// The machine's PS/2 mouse, as a host driving it sees it.
pub struct Mouse<'m, T: Instrumentation> {
    machine: &'m mut Emulator<T>,
}

impl<T: Instrumentation> Mouse<'_, T> {
    /// Report relative motion and the current button mask (bit 0 left, bit 1
    /// right, bit 2 middle), returning whether a packet reached the guest.
    ///
    /// A `false` is usually not backpressure: the guest may have the mouse in
    /// remote mode or reporting disabled, or nothing may have changed. Motion
    /// refused for any of those reasons accumulates and folds into the next
    /// packet, because PS/2 deltas are relative — a button edge does not, which
    /// is the case worth checking for.
    #[must_use = "a refused packet carried a button edge the guest will never see"]
    pub fn motion(&mut self, dx: i32, dy: i32, dz: i32, buttons: u8) -> bool {
        self.machine
            .device_manager
            .keyboard
            .mouse_motion(dx, dy, dz, buttons)
    }
}

/// Why a batch of execution ended.
///
/// Exhaustive over what `step_batch` can actually determine at the moment it
/// returns. Every variant here is a condition the loop genuinely distinguishes;
/// a cause the machine cannot tell apart from another does not get a name,
/// because a caller matching on it would be matching on a guess.
///
/// The order matters where causes coincide — a guest that powers off on the
/// same batch that exhausts its budget is reported as powering off, because the
/// budget will still be there next call and the power-off will not.
///
/// Closed on purpose (R5). The reserved breakpoint cause the debugger seam
/// will add is precisely the kind a caller must not silently ignore, so
/// adding it should be a compile error everywhere a stop is dispatched.
/// [`BatchOutcome::is_terminal`] is there for callers who only need the
/// boolean and should not have to enumerate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StopReason {
    /// The guest asked to be powered off: ACPI `PM1_CNT` with `SLP_EN` and
    /// `SLP_TYP` = S5, or the port-0x8900 shutdown protocol. Bochs treats both
    /// as `bx_user_quit` and aborts; this machine stops gracefully instead, so
    /// a caller that keeps stepping is running a guest that asked to be off.
    GuestPowerOff,
    /// The host asked, through `StopHandle::stop`, `Emulator::emu_stop`, or a
    /// shared stop flag installed with `set_stop_flag`.
    StopRequested,
    /// The boot CPU is in the architectural shutdown state. A triple fault is
    /// the usual way in; `RSM` with an inconsistent SMRAM image and a VMX entry
    /// carrying guest activity state 2 reach the same state, and the CPU
    /// records no cause, so this variant does not claim to know which.
    CpuShutdown,
    /// The boot CPU is halted (`HLT`/`MWAIT`) or waiting for a `SIPI`, and no
    /// wake event arrived within the idle fast-forward budget. Time still
    /// advanced; the caller may step again to keep advancing it.
    Halted,
    /// The batch ran out of budget with the CPU still executing. Under `std`
    /// that is the 15 ms wall-clock budget, not `batch_instructions` — see
    /// `Emulator::step_batch`.
    BudgetExhausted,
}

/// What one `step_batch` call did.
///
/// A named struct rather than a pair (doctrine R0): the two fields are a count
/// and a cause, and nothing about `(u64, bool)` said which was which — nor
/// could a `bool` carry more than one of the five causes above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BatchOutcome {
    /// Instructions retired across the whole call. Under `std` this may exceed
    /// the requested `batch_instructions`, because the call keeps running whole
    /// batches until its wall-clock budget is spent.
    pub executed: u64,
    /// Why the call returned.
    pub stop: StopReason,
}

impl BatchOutcome {
    /// Whether the machine should not be stepped again without host action.
    ///
    /// `Halted` and `BudgetExhausted` are ordinary yields — the guest is still
    /// live and stepping again makes progress. The other three mean the guest
    /// or the host has asked execution to end, and a caller that keeps stepping
    /// is spinning.
    #[inline]
    pub fn is_terminal(&self) -> bool {
        match self.stop {
            StopReason::GuestPowerOff | StopReason::StopRequested | StopReason::CpuShutdown => true,
            StopReason::Halted | StopReason::BudgetExhausted => false,
        }
    }
}

/// Why the machine's stop flag is raised.
///
/// The flag itself is a shared `AtomicBool` a host thread may own, so it cannot
/// carry this: a host setting it through `StopHandle` writes only the bool.
/// The machine records the cause beside it when *it* is the one raising the
/// flag, and reads "host asked" from the absence of a recorded guest cause.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum StopCause {
    /// Nothing the machine itself raised — so if the flag is up, the host did it.
    #[default]
    HostRequested,
    /// The guest executed an ACPI S5 transition or the port-0x8900 protocol.
    GuestPowerOff,
}

impl<'a, T: Instrumentation> Emulator<T> {
    #[cfg(feature = "alloc")]
    /// Draw one VGA frame to the attached front end.
    ///
    /// Bochs `bx_vgacore_c::update()` calls the GUI directly; so does this, by
    /// handing the card a sink over the `BxGui` this machine holds. What used
    /// to live here — the clear-screen and palette drains, the charmap push,
    /// the cursor arithmetic and the two shapes of frame — is inside the card
    /// now, where upstream keeps it, and is shared with every other front end.
    pub fn update_gui(&mut self) {
        let Some(ref mut gui) = self.gui else {
            return;
        };
        let mut sink = crate::gui::gui_trait::GuiSink::new(&mut **gui);
        // Deliberately not consulted here: a `BxGui` presents whatever it was
        // given when the refresh flushes, so this pump has nothing to decide.
        // The buffer-rendering path does — see `Display::render_into`.
        let _presented_unconditionally = self.device_manager.vga.refresh(&mut sink);
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
        let ram_size = self.config.memory.guest_bytes() as u64;
        let cpu_count = self.config.cpu_params.cpu_count();

        // Shared implementation in `crate::boot` — one boot path serves both
        // the alloc Emulator and the raw (cpu, memory) no-alloc entry point.
        //
        // The boot CPU and memory are needed at once, which no single accessor
        // can hand out, so they come from one destructuring of disjoint fields —
        // the same shape `exec_ctx` uses.
        let Self { cpus, memory, .. } = self;
        crate::boot::setup_direct_linux_boot_with_pins(
            cpus.get_mut(0),
            memory,
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
    /// caller must yield control back to its event loop regularly. Runs the
    /// guest, ticks devices, syncs A20, then returns why it stopped.
    ///
    /// `batch_instructions` bounds one inner batch, not the call. Under `std` the
    /// call keeps running batches until a 15 ms wall-clock budget is spent — so
    /// `BatchOutcome::executed` may come back well above `batch_instructions`.
    /// Under `no_std` there is no clock, so the instruction count is the budget
    /// and the figure is a true ceiling.
    ///
    /// A caller driving a machine to completion should stop on
    /// [`BatchOutcome::is_terminal`]; testing the count against
    /// `batch_instructions` cannot tell "the guest powered off" from "the budget
    /// ran out", and under `std` cannot even tell the budget ran out.
    pub fn step_batch(&mut self, batch_instructions: u64) -> Result<BatchOutcome> {
        let ips = self.config.ips.per_second_u64();
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
            let result = self.run_cpu_batch(batch_instructions);

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
                self.cpu_ref(0).activity_state,
                CpuActivityState::Hlt | CpuActivityState::Mwait | CpuActivityState::MwaitIf
            ) && self.can_fast_forward_bsp_hlt()
            {
                let mwait_if = matches!(self.cpu_ref(0).activity_state, CpuActivityState::MwaitIf);
                let mut hlt_budget = 0u64;
                while hlt_budget < 100_000_000 {
                    // Service host input while halted (see run_interactive).
                    self.pump_gui_input();
                    if self.has_interrupt() && (self.cpu_ref(0).interrupts_enabled() || mwait_if) {
                        break;
                    }
                    self.service_lapic_local_events();
                    if self.cpu_ref(0).lapic.intr && (self.cpu_ref(0).interrupts_enabled() || mwait_if) {
                        self.cpu_mut()
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
                if self.cpu_ref(0).lapic_has_intr() {
                    self.cpu_mut()
                        .signal_event(BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR);
                }
            }

            // --- Deliver PIC interrupt ---
            if self.device_manager.has_interrupt()
                && self.cpu_ref(0).get_b_if() != 0
                && !self.cpu_ref(0).interrupts_inhibited(0x01)
                // Bochs event.cc delivers Priority-4 debug traps before
                // Priority-5 external interrupts; defer one boundary so the
                // CPU can deliver #DB first. The interrupt stays latched in
                // the PIC because iac() is not called.
                && !self.cpu_ref(0).debug_trap_pending()
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
            // A raised stop flag ends the call now rather than at the end of
            // the wall-clock budget. It is raised by a host `StopHandle`, by a
            // guest power-off drained at a scheduler boundary, and by a hook
            // whose honoured stop request the batch just drained — none of
            // which is worth spending another 15 ms of guest time on.
            if !self.stop_flag.load(core::sync::atomic::Ordering::Relaxed)
                && matches!(self.cpu_ref(0).activity_state, CpuActivityState::Active)
                && {
                    #[cfg(feature = "std")]
                    {
                        wall_start.elapsed() < wall_budget
                    }
                    #[cfg(not(feature = "std"))]
                    {
                        total_executed < batch_instructions
                    }
                }
            {
                continue 'batch;
            }

            break 'batch;
        }


        // Handle keyboard/mouse/serial input from GUI.
        self.pump_gui_input();

        Ok(BatchOutcome {
            executed: total_executed,
            stop: self.classify_batch_stop(),
        })
    }

    /// Execute at most `instructions` guest instructions, and no more.
    ///
    /// The strict counterpart to [`Self::step_batch`]. That one treats its
    /// argument as one inner batch and keeps going for a 15 ms wall-clock
    /// budget, which is right for throughput and wrong for anything that has
    /// to look at the machine between instructions: an address to stop at, a
    /// single step, a debugger. This one runs the count and returns, with no
    /// wall clock and no HLT fast-forward past it.
    ///
    /// Devices still advance by the ticks the CPU consumed, so guest time does
    /// not fall behind — only the batching is different.
    #[cfg(feature = "alloc")]
    pub(crate) fn step_exactly(&mut self, instructions: u64) -> Result<BatchOutcome> {
        let executed = self
            .run_cpu_batch_with_strict_limit(instructions, true)
            .map_err(crate::error::Error::Cpu)?;
        if !self.batch_advanced_pc_system {
            self.advance_pc_system_after_cpu_ticks(executed);
        }
        self.pump_gui_input();
        Ok(BatchOutcome {
            executed,
            stop: self.classify_batch_stop(),
        })
    }

    /// The machine's power and reset controls.
    pub fn power(&mut self) -> Power<'_, T> {
        Power { machine: self }
    }

    /// The machine's keyboard, for a host driving it.
    pub fn keyboard(&mut self) -> Keyboard<'_, T> {
        Keyboard { machine: self }
    }

    /// The machine's PS/2 mouse, for a host driving it.
    pub fn mouse(&mut self) -> Mouse<'_, T> {
        Mouse { machine: self }
    }

    /// How many ticks of guest time may pass before the next device timer
    /// fires, or `None` when no timer is armed.
    ///
    /// The question a caller asks to avoid grinding through idle instructions:
    /// a machine waiting on a PIT or RTC deadline retires nothing interesting
    /// until it arrives, so a driver can size its next `step_batch` from this
    /// instead of stepping blindly and checking afterwards. QEMU's qtest
    /// exposes the same thing as its most-used verb, `clock_step` with no
    /// argument; this is the query form, leaving the caller to decide how far
    /// to actually run.
    ///
    /// Zero means a deadline is due now. It shares its computation with the
    /// scheduler's own deadline cap, so the two cannot disagree about when the
    /// next event is.
    pub fn ticks_to_next_timer_deadline(&self) -> Option<u64> {
        self.pc_system.ticks_to_next_timer_deadline()
    }

    /// Why the batch loop above just ended.
    ///
    /// Asked once, after the loop, from state that is still live — which is
    /// what makes the answer honest. The order is by precedence, not by
    /// likelihood: a guest that powers off on the same call that fills its
    /// budget has to be reported as powering off, because the budget renews on
    /// the next call and the power-off request does not.
    fn classify_batch_stop(&mut self) -> StopReason {
        if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) {
            return match self.stop_cause {
                StopCause::GuestPowerOff => StopReason::GuestPowerOff,
                StopCause::HostRequested => StopReason::StopRequested,
            };
        }
        // The flag is down, so nothing the machine recorded about a previous
        // raise still applies. Clearing here is what keeps a stale guest cause
        // from being read back the next time a HOST holder of the shared flag
        // raises it — that holder writes the bool and nothing else.
        self.stop_cause = StopCause::HostRequested;

        if self.cpu_ref(0).is_in_shutdown() {
            return StopReason::CpuShutdown;
        }
        // "Halted" is a property of the MACHINE, not of the boot CPU: on an SMP
        // guest the boot CPU may sit in `HLT` while an application processor
        // does the work, and that batch ended on budget with progress still
        // available. Asked of every CPU rather than read off `runnable_mask`,
        // which is a cache the scheduler maintains during a batch.
        let progress_possible = (0..self.cpu_count()).any(|i| self.cpu_runnable_for_batch(i));
        if !progress_possible {
            return StopReason::Halted;
        }
        StopReason::BudgetExhausted
    }

}

/// Render one VGA frame into a `SharedDisplay` framebuffer.
///
/// The same refresh `update_gui` performs, with the shared framebuffer as the
/// sink instead of a `BxGui`. It used to be a second hand-maintained copy of
/// that forwarding code, which is why it never forwarded the character
/// generators: a guest that reprogrammed the font rendered with stale glyphs
/// here and nowhere else. One pump, one sink, so the two cannot drift again.
/// Reached through [`crate::emulator::Display::render_into`].
#[cfg(feature = "alloc")]
pub(crate) fn render_vga_into(
    vga: &mut crate::iodev::vga::BxVgaC,
    display: &mut crate::gui::shared_display::SharedDisplay,
) -> crate::iodev::display_sink::Refreshed {
    vga.refresh(display)
}