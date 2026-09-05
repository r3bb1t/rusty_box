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

use super::{Emulator, EventDelivery, ProgressUnit, ResetReason, SliceEngine, SoftwareEngine};
use crate::memory::plan::{MemoryPlan, MemoryPlanError};
// Only the direct-Linux-boot path below reaches a CPU through the store, and
// that path needs an allocator.
#[cfg(feature = "alloc")]
use super::cpu_store::CpuStore;

/// What a machine was asked for that its engine cannot do.
///
/// Separate from the faults a run can hit, because nothing went wrong: the ask
/// was answerable only by a different engine, and the machine says so instead
/// of running and reporting a number that means nothing.
///
/// `#[non_exhaustive]` at birth: engines are written outside this crate, so the
/// set of asks one can decline grows with the seam rather than with a release
/// of this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EngineRefusal {
    /// An instruction budget, on an engine that reports guest time.
    ///
    /// The budget would never be spent: nothing this machine can measure
    /// advances, so the run would not end until the guest did. Ask in
    /// [`RunBudget::Ticks`] instead — the unit both engines hold to.
    #[error(
        "this machine's engine reports guest time, not retired instructions, so an instruction budget could never be spent"
    )]
    NoInstructionCount,
}

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
pub struct Power<'m, T: Instrumentation, E = SoftwareEngine> {
    machine: &'m mut Emulator<T, E>,
}

impl<T: Instrumentation, E: SliceEngine<T>> Power<'_, T, E> {
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
    ///
    /// Takes `&mut self` because reading why the machine stopped is what
    /// retires a cause whose raise no longer stands — see
    /// [`Emulator::stop_in_force`]. A faulted machine reads `Running`: it is
    /// powered on and stopped, which is not the same as switched off.
    pub fn state(&mut self) -> PowerState {
        if self.machine.cpu_ref(0).is_in_shutdown() {
            return PowerState::CpuShutdown;
        }
        if self.machine.stop_in_force() == Some(StopCause::GuestPowerOff) {
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
///
/// Built from the device, not from the machine. Every verb below reaches the
/// 8042 and nothing else, so borrowing the whole machine would tie the handle
/// to how that machine is parameterised for no reason — and a machine on any
/// engine hands out this same type. `Display` already has this shape.
pub struct Keyboard<'m> {
    keyboard: &'m mut crate::iodev::keyboard::BxKeyboardC,
}

impl<'m> Keyboard<'m> {
    pub(crate) fn new(keyboard: &'m mut crate::iodev::keyboard::BxKeyboardC) -> Self {
        Self { keyboard }
    }

    /// Press or release a key, rendered through the guest's active scancode
    /// set. Returns whether the whole sequence reached the guest — see
    /// `BxKeyboardC::gen_scancode` for why a key can be partly delivered.
    #[must_use = "a refused key never reached the guest"]
    pub fn key(&mut self, key: crate::iodev::scancodes::BxKey, pressed: bool) -> bool {
        self.keyboard.gen_scancode(key, pressed)
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
            if !self.keyboard.send_scancode(byte) {
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
///
/// Built from the device for the same reason as [`Keyboard`], and from the
/// same device: on a PC the mouse hangs off the 8042's auxiliary port.
pub struct Mouse<'m> {
    keyboard: &'m mut crate::iodev::keyboard::BxKeyboardC,
}

impl<'m> Mouse<'m> {
    pub(crate) fn new(keyboard: &'m mut crate::iodev::keyboard::BxKeyboardC) -> Self {
        Self { keyboard }
    }

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
        self.keyboard.mouse_motion(dx, dy, dz, buttons)
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
    /// The machine's engine refused something the machine cannot do itself —
    /// an I/O APIC message its backend would not take, an interrupt edge it
    /// could not be told about — and the machine stopped rather than run a
    /// guest waiting on an interrupt nothing will deliver.
    ///
    /// The fault itself travels as `CpuError::EngineFault` from the boundary
    /// that heard it. This is what a caller sees when the boundary's error had
    /// nowhere to go, which is the case on every path that only logs one.
    EngineFault,
}

/// What bounds one run.
///
/// The two units a machine can hold itself to without consulting a clock it
/// does not own. A host deadline is the third, and waits for a machine that has
/// a `HostClock`: `std::time::Instant` is not available on every target this
/// runs on, and a front end that wants to bound a frame already has one.
///
/// Both bounds are honoured exactly. Neither is a hint about an inner batch —
/// that shape is what let `step_batch(1)` retire 475,135 instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunBudget {
    /// Guest instructions the boot processor may retire.
    ///
    /// Counted on the boot processor because that is the one a caller set RIP
    /// on. An engine that does not count instructions cannot honour this.
    Instructions(u64),
    /// Ticks of guest time to let pass.
    ///
    /// The unit both a multiprocessor machine and a hardware engine can hold
    /// to, and the one [`Emulator::ticks_to_next_timer_deadline`] answers in —
    /// so "run until the next device deadline" is expressible without guessing
    /// how many instructions that is.
    Ticks(u64),
}

/// How far a run got, in the unit the machine can actually answer in.
///
/// A uniprocessor answers in instructions: one processor retires them, and its
/// count is the machine's own. A multiprocessor cannot. Bochs `main.cc` gives
/// every processor a quantum and credits the whole round with one `BX_TICKN`,
/// so machine time advances by the round's average — a figure no single
/// processor's instruction count describes, and one that keeps moving while a
/// halted processor retires nothing at all.
///
/// The two used to travel as one `u64` called `executed`, documented as
/// instructions and holding ticks whenever the machine had more than one
/// processor. Naming them apart is doctrine R4: a count of instructions and a
/// span of time are different things, and a caller that adds them is wrong in
/// a way no type was previously able to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Guest instructions retired.
    Instructions(u64),
    /// Ticks of guest time that passed.
    Ticks(u64),
}

impl Progress {
    /// The instructions retired, or `None` when the machine measured its
    /// progress as time instead.
    #[inline]
    #[must_use]
    pub fn instructions(self) -> Option<u64> {
        match self {
            Self::Instructions(count) => Some(count),
            Self::Ticks(_) => None,
        }
    }

    /// The guest time that passed, or `None` when the machine measured its
    /// progress as instructions instead.
    #[inline]
    #[must_use]
    pub fn ticks(self) -> Option<u64> {
        match self {
            Self::Ticks(count) => Some(count),
            Self::Instructions(_) => None,
        }
    }

    /// Whether the guest failed to advance at all.
    ///
    /// One of the two questions both units answer the same way, and the one a
    /// caller asks to find out whether stepping again is worth it.
    #[inline]
    #[must_use]
    pub fn stalled(self) -> bool {
        match self {
            Self::Instructions(count) | Self::Ticks(count) => count == 0,
        }
    }

    /// Add a later run's progress to this one.
    ///
    /// A machine's processor count is fixed at construction, so every run of
    /// one machine reports the same unit. Two that disagree would mean adding
    /// a duration to a count, which is refused rather than summed: the caller
    /// is handed back the later reading, and the machine has a defect no total
    /// could describe.
    #[inline]
    #[must_use]
    pub(crate) fn add(self, later: Self) -> Self {
        match (self, later) {
            (Self::Instructions(a), Self::Instructions(b)) => {
                Self::Instructions(a.saturating_add(b))
            }
            (Self::Ticks(a), Self::Ticks(b)) => Self::Ticks(a.saturating_add(b)),
            (before, later) => {
                tracing::error!(
                    "a machine changed how it measures progress mid-run: {before:?} then {later:?}"
                );
                later
            }
        }
    }

    /// The number, with the unit deliberately discarded.
    ///
    /// For pacing, where the unit genuinely does not matter: a front end
    /// bounding how much it does per frame wants to know how much happened,
    /// not what kind. Anything that will compare against an instruction count
    /// or a deadline must ask [`Self::instructions`] or [`Self::ticks`] and
    /// handle the `None` — discarding the unit there is how the two got
    /// confused in the first place.
    #[inline]
    #[must_use]
    pub const fn count(self) -> u64 {
        match self {
            Self::Instructions(count) | Self::Ticks(count) => count,
        }
    }
}

/// What one run did.
///
/// A named struct rather than a pair (doctrine R0): the two fields are how far
/// it got and why it stopped, and nothing about `(u64, bool)` said which was
/// which — nor could a `bool` carry more than one of the five causes above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BatchOutcome {
    /// How far the guest got. Under `std` this may exceed what was asked for,
    /// because the call keeps running whole batches until its wall-clock
    /// budget is spent.
    pub progress: Progress,
    /// Why the call returned.
    pub stop: StopReason,
}

impl BatchOutcome {
    /// Whether the machine should not be stepped again without host action.
    ///
    /// `Halted` and `BudgetExhausted` are ordinary yields — the guest is still
    /// live and stepping again makes progress. The other four mean execution
    /// has ended: the guest or the host asked, the processor is in shutdown, or
    /// the engine refused work the machine cannot do without it. A caller that
    /// keeps stepping past any of them is spinning.
    #[inline]
    pub fn is_terminal(&self) -> bool {
        match self.stop {
            StopReason::GuestPowerOff
            | StopReason::StopRequested
            | StopReason::CpuShutdown
            | StopReason::EngineFault => true,
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
    /// The machine's engine refused work the machine cannot do without it, so
    /// the boundary that heard the refusal stopped the machine beneath it.
    EngineFault,
}

impl StopCause {
    /// Whether this cause takes the field from `recorded`, when both describe
    /// the same raised flag.
    ///
    /// One boundary can produce two of these: a guest power-off is drained at
    /// its head and an engine refusal is surfaced at its tail. One field holds
    /// one answer, so the order is fixed here rather than by which write ran
    /// last.
    ///
    /// A guest power-off outranks everything. It is a terminal fact about the
    /// machine and it is consumed where it is drained — no later boundary can
    /// rediscover it, and a caller that is told anything else keeps a machine
    /// alive that the guest switched off. An engine refusal loses nothing by
    /// yielding: it is returned whole on the same call, and the work it refused
    /// is still owed. A refused pin edge is offered again at the next boundary,
    /// which runs the tail unconditionally; a refused I/O APIC delivery is put
    /// back in the IRR and offered again at the next assertion of that line or
    /// the next redirection-entry write, because nothing re-runs the delivery
    /// path per boundary. A refusal in turn
    /// outranks a bare host request, which is the absence of a machine cause
    /// rather than a cause of its own.
    const fn displaces(self, recorded: Self) -> bool {
        match (recorded, self) {
            (Self::GuestPowerOff, _) => false,
            (_, Self::GuestPowerOff) => true,
            (Self::HostRequested, Self::EngineFault) => true,
            (Self::HostRequested, Self::HostRequested)
            | (Self::EngineFault, Self::EngineFault)
            | (Self::EngineFault, Self::HostRequested) => false,
        }
    }
}

impl From<StopCause> for StopReason {
    /// The one place the machine's private vocabulary for "why did the flag go
    /// up" becomes the one its callers read (R5), so the two cannot drift.
    fn from(cause: StopCause) -> Self {
        match cause {
            StopCause::HostRequested => Self::StopRequested,
            StopCause::GuestPowerOff => Self::GuestPowerOff,
            StopCause::EngineFault => Self::EngineFault,
        }
    }
}

impl<'a, T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
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
        // Self-tracked: this engine executes every guest write itself, so the
        // card's own tile bitmap is the complete record of what changed. A
        // hypervisor engine, whose framebuffer window the guest writes without
        // exiting, supplies its page bitmap here instead.
        let _presented_unconditionally = self
            .device_manager
            .vga
            .refresh(&mut sink, rusty_box_devices::display::card::Dirt::SelfTracked);
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
        // that resumes at the reset vector either way. A failure has nowhere to
        // go from an input pump the idle waits call for responsiveness, so it
        // is logged — and the boundary that raised it raised the machine's stop
        // flag with it, so the loop that called this ends on that instead.
        if let Err(error) = self.service_scheduler_boundary(0) {
            tracing::error!("host-input scheduler boundary failed: {error}");
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
            let pic = self.device_manager.irq.pic_mut();
            // Initialize master PIC: ICW1-ICW4
            // ICW1: edge-triggered, cascade, ICW4 needed
            pic.write(0x20, 0x11, 1);
            // ICW2: master vectors 0x20-0x27 (Linux kernel expects IRQ0=0x20)
            pic.write(0x21, 0x20, 1);
            // ICW3: slave on IRQ2
            pic.write(0x21, 0x04, 1);
            // ICW4: 8086 mode, normal EOI
            pic.write(0x21, 0x01, 1);
            // OCW1: mask all master IRQs — kernel will unmask what it needs
            pic.write(0x21, 0xFF, 1);

            // Initialize slave PIC: ICW1-ICW4
            pic.write(0xA0, 0x11, 1);
            // ICW2: slave vectors 0x28-0x2F (Linux kernel expects IRQ8=0x28)
            pic.write(0xA1, 0x28, 1);
            // ICW3: cascade identity = 2
            pic.write(0xA1, 0x02, 1);
            // ICW4: 8086 mode
            pic.write(0xA1, 0x01, 1);
            // OCW1: mask all slave IRQs
            pic.write(0xA1, 0xFF, 1);

            // Do NOT program PIT — kernel will set up its own timer via time_init().
            // quick_pit_calibrate() programs PIT C2 via port 0x43/0x42 directly.
            tracing::debug!(
                "Direct boot: PIC initialized (master=0x20, slave=0x28), all IRQs masked"
            );
        }

        Ok(())
    }

    /// Run the guest until `budget` is spent, then hand control back.
    ///
    /// Cooperative: the call returns, so a single-threaded host — wasm, UEFI, a
    /// GUI frame loop — keeps its event loop. Runs the guest, ticks devices,
    /// syncs A20, then says why it returned.
    ///
    /// The budget is a ceiling and is honoured exactly. Its predecessor's
    /// argument bounded an *inner* batch while the call ran on for a fixed
    /// 15 ms of host time, so asking for one instruction could retire hundreds
    /// of thousands of them, and asking twice on a loaded machine gave two
    /// different answers. A caller that wants to bound host time owns a clock
    /// and can bound its own loop; the machine no longer guesses on its behalf.
    ///
    /// A caller driving a machine to completion should stop on
    /// [`BatchOutcome::is_terminal`]: comparing progress against the budget
    /// cannot tell "the guest powered off" from "the budget ran out".
    ///
    /// # Errors
    /// [`EngineRefusal::NoInstructionCount`] when an instruction budget is
    /// asked of an engine that reports guest time. Otherwise whatever ended
    /// the run other than a budget or a stop: a fault the processor could not
    /// take, or a machine boundary that failed to apply.
    pub fn step(&mut self, budget: RunBudget) -> Result<BatchOutcome> {
        if matches!(budget, RunBudget::Instructions(_)) {
            Self::require_an_instruction_count()?;
        }
        // What the budget is measured against, sampled before anything runs.
        // Instructions come off the boot processor — the one whose RIP a caller
        // set — and ticks off the machine's own clock.
        let started_at = match budget {
            RunBudget::Instructions(_) => self.cpu_ref(0).icount,
            RunBudget::Ticks(_) => self.pc_system.time_ticks(),
        };
        let spent = |machine: &Self| match budget {
            RunBudget::Instructions(_) => {
                machine.cpu_ref(0).icount.saturating_sub(started_at)
            }
            RunBudget::Ticks(_) => machine.pc_system.time_ticks().saturating_sub(started_at),
        };
        let ceiling = match budget {
            RunBudget::Instructions(count) | RunBudget::Ticks(count) => count,
        };
        self.run_until_budget_spent(ceiling, spent)
    }

    /// The run loop, over a budget it can measure but need not understand.
    fn run_until_budget_spent(
        &mut self,
        ceiling: u64,
        spent: impl Fn(&Self) -> u64,
    ) -> Result<BatchOutcome> {
        let ips = self.config.ips.per_second_u64();
        // Unset until the first batch answers, so the running total never
        // guesses a unit: a multiprocessor answers in ticks, a uniprocessor in
        // whatever its engine counts, and a run that never got to execute
        // anything reports the nothing both units agree on.
        let mut total: Option<Progress> = None;
        // Consecutive batches that advanced nothing. See the check below.
        let mut stalled_batches = 0u32;
        // One inner batch is capped so a run that ends on a device deadline
        // still notices promptly; the budget above, not this, decides when the
        // call returns.
        const INNER_BATCH_CEILING: u64 = 100_000;

        'batch: loop {
            let remaining = ceiling.saturating_sub(spent(self));
            if remaining == 0 {
                break 'batch;
            }

            // How much to ask for at once. What an early return costs depends
            // entirely on the engine: an interpreter pays a trace boundary,
            // which is nothing, while an engine running the guest on real
            // hardware pays a whole architectural state exchange in each
            // direction — every register, every segment, every descriptor
            // table — and a ceiling sized for the first is ruinous for the
            // second. Measured: at 300 MHz the interpreter's 100,000-tick
            // ceiling is 333 microseconds of a processor that could have run
            // until the next device deadline, so a boot spends thousands of
            // exchanges a second and the hardware's speed goes with them.
            //
            // A hardware engine is handed the whole remaining budget, because
            // `run_cpu_batch` already clamps a batch to the next device
            // deadline — which was the only reason to cut one short.
            let ask = match E::PROGRESS_UNIT {
                ProgressUnit::Instructions => remaining.min(INNER_BATCH_CEILING),
                ProgressUnit::Ticks => remaining,
            };

            // --- Run CPU batch ---
            let progress = match self.run_cpu_batch_with_strict_limit(ask, true) {
                Ok(progress) => progress,
                Err(e) => return Err(crate::error::Error::Cpu(e)),
            };
            total = Some(match total {
                Some(so_far) => so_far.add(progress),
                None => progress,
            });

            // --- Tick devices + pc_system ---
            // Only reached when the batch advanced nothing itself, which on a
            // multiprocessor it always does — so in practice this is the
            // uniprocessor path, where a retired instruction is a tick.
            if !self.batch_advanced_pc_system {
                self.advance_pc_system_after_cpu_ticks(progress.count())
                    .map_err(crate::error::Error::Cpu)?;
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
            // Only when delivery is the machine's. An engine that delivers at
            // the head of its own stretches has already taken this vector off
            // the 8259 by the time control returns here, and `iac()` cannot
            // give it back: injecting again hands the guest an interrupt no
            // driver is waiting for.
            if matches!(E::EVENT_DELIVERY, EventDelivery::Machine)
                && self.device_manager.has_interrupt()
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

            // Keep going while the guest is live and the budget is not spent.
            //
            // A raised stop flag ends the call at once rather than at the end
            // of the budget. It is raised by a host `StopHandle`, by a guest
            // power-off drained at a scheduler boundary, and by a hook whose
            // honoured stop request the batch just drained — none of which is
            // worth spending more guest time on. A processor that is no longer
            // Active has had its chance to wake in the fast-forward above; if
            // it did not, returning lets the caller decide what to do about it.
            // A budget is spent by the machine making progress, so a machine
            // that has stopped making any cannot spend one. Nothing here can
            // change between two identical empty batches, so asking a third
            // time is a livelock, and this loop has no other way out: the
            // budget is measured against a clock the batches were supposed to
            // advance.
            //
            // Two rather than one, because a single empty batch is ordinary —
            // a boundary that consumed the whole round, a reset applied at
            // entry — and only a repeat means nothing is coming.
            if progress.stalled() {
                stalled_batches += 1;
                if stalled_batches >= 2 {
                    tracing::warn!(
                        "a batch advanced nothing twice running; ending the call rather \
                         than spending a budget the machine cannot spend"
                    );
                    break 'batch;
                }
            } else {
                stalled_batches = 0;
            }

            if !self.stop_flag.load(core::sync::atomic::Ordering::Relaxed)
                && matches!(self.cpu_ref(0).activity_state, CpuActivityState::Active)
            {
                continue 'batch;
            }

            break 'batch;
        }


        // Handle keyboard/mouse/serial input from GUI.
        self.pump_gui_input();

        Ok(BatchOutcome {
            // A run that never reached a batch advanced nothing, and no
            // instructions is the reading both units spell the same way.
            progress: total.unwrap_or(Progress::Instructions(0)),
            stop: self.classify_batch_stop(),
        })
    }

    /// The one place a machine tests whether an ask denominated in
    /// instructions is answerable at all (R5).
    ///
    /// Reads a constant of the engine type, so a machine on a counting engine
    /// pays nothing for it — the branch folds away at monomorphisation and the
    /// error is unreachable.
    ///
    /// # Errors
    /// [`EngineRefusal::NoInstructionCount`] when the engine reports guest
    /// time instead.
    fn require_an_instruction_count() -> Result<()> {
        match E::PROGRESS_UNIT {
            ProgressUnit::Instructions => Ok(()),
            ProgressUnit::Ticks => Err(EngineRefusal::NoInstructionCount.into()),
        }
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
    ///
    /// # Errors
    /// [`EngineRefusal::NoInstructionCount`] on an engine that does not retire
    /// the guest's instructions itself: it cannot stop after a count it never
    /// sees, and running a whole stretch instead would be this method's exact
    /// promise broken.
    #[cfg(feature = "alloc")]
    pub(crate) fn step_exactly(&mut self, instructions: u64) -> Result<BatchOutcome> {
        Self::require_an_instruction_count()?;
        let progress = self
            .run_cpu_batch_with_strict_limit(instructions, true)
            .map_err(crate::error::Error::Cpu)?;
        if !self.batch_advanced_pc_system {
            self.advance_pc_system_after_cpu_ticks(progress.count())
                .map_err(crate::error::Error::Cpu)?;
        }
        self.pump_gui_input();
        Ok(BatchOutcome {
            progress,
            stop: self.classify_batch_stop(),
        })
    }

    /// The engine this machine runs its guest on.
    ///
    /// A machine's engine is chosen when it is built and never changes, so
    /// this is how a caller that named one asks it something afterwards — how
    /// a hypervisor partition is faring, what a test engine recorded. Shared
    /// rather than exclusive: running the guest is the machine's to do, and an
    /// engine driven from two places would be two machines.
    #[must_use]
    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// The machine's power and reset controls.
    pub fn power(&mut self) -> Power<'_, T, E> {
        Power { machine: self }
    }

    /// The machine's keyboard, for a host driving it.
    pub fn keyboard(&mut self) -> Keyboard<'_> {
        Keyboard::new(&mut self.device_manager.keyboard)
    }

    /// The machine's PS/2 mouse, for a host driving it.
    pub fn mouse(&mut self) -> Mouse<'_> {
        Mouse::new(&mut self.device_manager.keyboard)
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
    /// The machine's guest-physical map, as an execution engine would install
    /// it.
    ///
    /// The complement of stepping: `step_batch` asks the machine to run, this
    /// asks it where its memory *is*. An engine that executes the guest on real
    /// hardware needs the whole map up front rather than one address at a time,
    /// and re-derives it whenever the chipset moves something — a shadow-RAM
    /// flip, a relocated BAR.
    ///
    /// A range absent from the map is not a hole in the guest's address space.
    /// It is a range this machine services itself: device MMIO, the local APIC
    /// page, the video aperture, unbacked memory. An engine leaves the guest on
    /// such an access and hands it back.
    ///
    /// # Errors
    /// [`MemoryPlanError::PartiallyResident`] when the machine was built with
    /// less host memory than guest memory. There is no stable map in that
    /// regime: the residency map moves guest blocks between host slots as the
    /// guest touches them.
    pub fn memory_plan(&self) -> core::result::Result<MemoryPlan, MemoryPlanError> {
        MemoryPlan::derive(&self.memory)
    }

    pub fn ticks_to_next_timer_deadline(&self) -> Option<u64> {
        self.pc_system.ticks_to_next_timer_deadline()
    }

    /// Stop the machine, recording why.
    ///
    /// The one place the flag goes up with a cause of the machine's own (R5) —
    /// a host request raises the bare flag instead, and is read back from the
    /// absence of a machine cause. Going through here is what keeps the cause
    /// honest when one boundary produces two of them: [`StopCause::displaces`]
    /// decides which survives, so neither write has to know what the other did
    /// or which of them ran last.
    ///
    /// A flag that is already down carries no cause — [`Self::stop_in_force`]
    /// retires the field whenever it finds the flag lowered — so the incoming
    /// cause simply takes it.
    pub(super) fn raise_stop(&mut self, cause: StopCause) {
        let cause_in_force = self.stop_flag.load(core::sync::atomic::Ordering::Relaxed);
        if !cause_in_force || cause.displaces(self.stop_cause) {
            self.stop_cause = cause;
        }
        self.stop_flag
            .store(true, core::sync::atomic::Ordering::Relaxed);
    }

    /// The machine cause in force right now, or `None` when the flag is down.
    ///
    /// The other half of [`Self::raise_stop`]'s choke point (R5): a cause means
    /// something only while the raise that recorded it still stands, so the one
    /// place that reads the field is also the one place that retires it. That
    /// pairing is what a shared flag makes necessary. A host holds it as a bare
    /// `&AtomicBool` and both lowers and raises it without touching the cause,
    /// so a cause left behind by a machine raise would otherwise be read back as
    /// the reason for the host's next bare one — reporting a guest power-off, or
    /// an engine fault, for a machine the host merely paused.
    ///
    /// Every reader goes through here — the batch loop's classifier, the
    /// device-time boundary a guest-off-thread driver runs, and
    /// [`Power::state`] — so any of them retires a spent cause for all of them.
    /// What that buys is bounded by reading rather than absolute. The flag is
    /// lowered in several places and none of them retires the cause — a host
    /// through its own clone, `emu_start` and `step_one` before they run — so a
    /// cause standing at a lower survives until the next read, and a raise that
    /// beats that read is answered with it; a bare `AtomicBool` carries nothing
    /// that could tell two raises apart. That is a bound on what a reader can
    /// be told, not on the machine: a driver that never asks — the interactive
    /// loop services boundaries and reads no cause at all — cannot be misled by
    /// one.
    pub(super) fn stop_in_force(&mut self) -> Option<StopCause> {
        if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) {
            return Some(self.stop_cause);
        }
        self.stop_cause = StopCause::HostRequested;
        None
    }

    /// Why the batch loop above just ended.
    ///
    /// Asked once, after the loop, from state that is still live — which is
    /// what makes the answer honest. The order is by precedence, not by
    /// likelihood: a guest that powers off on the same call that fills its
    /// budget has to be reported as powering off, because the budget renews on
    /// the next call and the power-off request does not.
    fn classify_batch_stop(&mut self) -> StopReason {
        if let Some(cause) = self.stop_in_force() {
            return cause.into();
        }

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
    vga: &mut rusty_box_devices::display::card::VgaCard<rusty_box_devices::display::card::StdVga>,
    display: &mut crate::gui::shared_display::SharedDisplay,
) -> rusty_box_devices::display::sink::Refreshed {
    vga.refresh(display, rusty_box_devices::display::card::Dirt::SelfTracked)
}