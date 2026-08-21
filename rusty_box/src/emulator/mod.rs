#![allow(unused_variables)]
//! Emulator Container
//!
//! This module provides the `Emulator` struct that owns and coordinates all
//! emulator components: CPU, Memory, Devices, and PC System.
//!
//! Each `Emulator` instance is fully independent with no global state,
//! allowing hundreds of emulator instances to run concurrently on different threads.

#[cfg(feature = "alloc")]
use crate::gui::BxGui;
#[cfg(feature = "std")]
use crate::pc_system::TimerOwner;
#[cfg(feature = "alloc")]
use crate::{
    cpu::builder::BxCpuBuilder, iodev::acpi_tables::AcpiTableGenerator, memory::MemoryError,
};
use crate::{
    cpu::{
        cpu::CpuActivityState,
        instrumentation::{ExitSet, Instrumentation},
        BxCpuC, CpuidFreq, ResetReason,
    },
    iodev::{
        devices::{DeviceManager, SystemControlPort},
        BxDevicesC,
    },
    memory::{BxMemC, BxMemoryStubC},
    params::BxParams,
    pc_system::BxPcSystemC,
    Result,
};

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::AtomicBool;

mod builder;
pub use builder::{AtaSlot, BootDevice, BootOrder, BuildError, DiskGeometry, MachineBuilder};
mod display;
pub use display::{
    Display, DisplaySource, Resolution, RowChars, TextGrid, TextPos, TextView,
};

pub mod cpu_store;
use cpu_store::CpuStore;
mod interactive;
mod run;
pub use run::{BatchOutcome, Keyboard, Mouse, Power, PowerState, StopReason};
pub(crate) use run::StopCause;
mod scheduler;
mod timers;

const BOCHS_APIC_BUS_ID_MASK: u32 = 0xFF;

/// Fixed-width CPU membership bitmap for the accepted 254-CPU topology.
///
/// These masks are the scheduler's authoritative no-allocation hot indexes;
/// full scans are reserved for initialization, reset, restore, and test oracles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CpuMask([u64; 4]);

impl CpuMask {
    #[inline]
    const fn bit(index: usize) -> Option<(usize, u64)> {
        if index < 256 {
            Some((index / 64, 1u64 << (index % 64)))
        } else {
            None
        }
    }

    #[inline]
    fn assign(&mut self, index: usize, enabled: bool) {
        let Some((word, bit)) = Self::bit(index) else {
            return;
        };
        if enabled {
            self.0[word] |= bit;
        } else {
            self.0[word] &= !bit;
        }
    }

    #[inline]
    fn count(self, limit: usize) -> usize {
        let limit = limit.min(256);
        let full_words = limit / 64;
        let tail_bits = limit % 64;
        let mut count = 0usize;
        for word in &self.0[..full_words] {
            count += word.count_ones() as usize;
        }
        if tail_bits != 0 {
            count += (self.0[full_words] & ((1u64 << tail_bits) - 1)).count_ones() as usize;
        }
        count
    }

    #[inline]
    fn next_set(self, from: usize, limit: usize) -> Option<usize> {
        let limit = limit.min(256);
        if from >= limit {
            return None;
        }

        let mut word_index = from / 64;
        let mut word = self.0[word_index] & (u64::MAX << (from % 64));
        loop {
            if word != 0 {
                let index = word_index * 64 + word.trailing_zeros() as usize;
                return (index < limit).then_some(index);
            }
            word_index += 1;
            if word_index * 64 >= limit {
                return None;
            }
            word = self.0[word_index];
        }
    }

    #[allow(dead_code)]
    #[inline]
    pub(crate) fn contains(&self, index: usize) -> bool {
        Self::bit(index)
            .map(|(word, bit)| self.0[word] & bit != 0)
            .unwrap_or(false)
    }
}

/// Emulated instructions per second: the rate that ties guest time to the
/// wall clock.
///
/// A rate, not a count, which is why it is a type (R4). Every guest-visible
/// clock is calibrated from it — the PIT, the ACPI PM timer, the TSC and
/// CPUID's frequency leaves all derive their tick rate from this one number —
/// so a machine given the wrong one has a guest whose sense of time is wrong
/// in a way no test of instruction results can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ips(u32);

impl Ips {
    /// Bochs config.cc `cpu: ips`. Upstream raised this from 4M to 50M
    /// because 4M badly under-reports a modern host, which makes every guest
    /// timeout fire early.
    pub const BOCHS_DEFAULT: Self = Self(50_000_000);

    pub const fn new(per_second: u32) -> Self {
        Self(per_second)
    }

    pub const fn per_second(self) -> u32 {
        self.0
    }

    /// The same rate, where the tick arithmetic needs 64-bit headroom.
    pub const fn per_second_u64(self) -> u64 {
        self.0 as u64
    }
}

impl Default for Ips {
    fn default() -> Self {
        Self::BOCHS_DEFAULT
    }
}

/// How much RAM a machine has, and how much of it stays resident.
///
/// One value, because the two numbers are only meaningful together: a guest
/// size raised without the host size is not a bigger machine, it is the same
/// machine running out of an overflow file, and that is a decision worth
/// making on purpose rather than by forgetting a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySize {
    guest_bytes: usize,
    host_bytes: usize,
}

impl MemorySize {
    /// `mib` mebibytes of guest RAM, all of it resident in host memory.
    pub const fn mib(mib: usize) -> Self {
        Self::bytes(mib * 1024 * 1024)
    }

    /// `bytes` of guest RAM, all of it resident in host memory.
    pub const fn bytes(bytes: usize) -> Self {
        Self {
            guest_bytes: bytes,
            host_bytes: bytes,
        }
    }

    /// A guest larger than the host memory backing it: blocks beyond
    /// `host_bytes` live in an overflow file and swap in on demand.
    ///
    /// Slower, and under `std` it needs somewhere to put that file — which is
    /// why it is a separate constructor rather than a field anyone can set by
    /// halves. Bochs calls the same arrangement `memory: host=`.
    pub const fn partially_resident(guest_bytes: usize, host_bytes: usize) -> Self {
        Self {
            guest_bytes,
            host_bytes,
        }
    }

    /// What the guest believes it has.
    pub const fn guest_bytes(self) -> usize {
        self.guest_bytes
    }

    /// What the host keeps resident.
    pub const fn host_bytes(self) -> usize {
        self.host_bytes
    }

    /// Whether this machine has to swap blocks through an overflow file.
    pub const fn swaps(self) -> bool {
        self.host_bytes < self.guest_bytes
    }
}

impl Default for MemorySize {
    /// Bochs `BX_DEFAULT_MEM_MEGS` — parity even in the default.
    fn default() -> Self {
        Self::mib(32)
    }
}

/// Emulator configuration
#[derive(Debug, Clone)]
pub struct EmulatorConfig {
    /// How much RAM the guest has, and how much of it the host keeps resident.
    pub memory: MemorySize,
    /// Memory block size for allocation
    pub memory_block_size: usize,
    /// The rate that calibrates emulated time against wall-clock time.
    pub ips: Ips,
    /// Enable PCI support
    pub pci_enabled: bool,
    /// Register the VGA adapter as a PCI device (`1234:1111`, class `0300`) so a
    /// guest KMS driver (Linux `bochs-drm`) can bind for a high-res framebuffer.
    /// Off by default; requires `pci_enabled`. Experimental.
    pub pci_vga: bool,
    /// CPU parameters
    pub cpu_params: BxParams,
    /// Enable sync=slowdown clock synchronization.
    /// When true, the emulator sleeps to match wall-clock time during active
    /// (non-HLT) execution with a GUI attached. Matches Bochs `clock: sync=slowdown`.
    /// Default: true (GUI), false (headless). Override with RUSTY_BOX_NOSYNC=1.
    pub sync_slowdown: bool,
    /// Advance the PIT and ACPI PM timer on host wall-clock time instead of
    /// emulated (icount) time — Bochs `clock: sync=realtime` (pit.cc reads
    /// bx_virt_timer with is_realtime). Default false = Bochs `sync=none`:
    /// device timers advance strictly with emulated time, so guest PIT/TSC
    /// calibration measures exactly the `ips` rate and boots are
    /// deterministic. (Previously rusty_box force-enabled this in std builds,
    /// which made PIT-based calibration measure wall-clock host throughput.)
    pub sync_realtime: bool,
    /// SMP scheduling quantum — Bochs `cpu: quantum=N` (config.cc
    /// BXPN_SMP_QUANTUM): maximum instructions a CPU executes before control
    /// returns to the round-robin scheduler; also caps SMP trace length
    /// (icache.cc). Range 1-32 (config.h BX_SMP_QUANTUM_MIN/MAX), default 16.
    /// Larger values cost interrupt-interleave granularity but sharply reduce
    /// per-slice overhead (32 ≈ single-CPU throughput on idle-heavy phases).
    /// Ignored with a single CPU.
    pub smp_quantum: u32,
    /// How CPU models report the CPUID frequency leaves 0x15/0x16 — Bochs
    /// `cpu: cpuid_freq=hardware|none|ips` (cpuid.cc get_freq_leaf_15/16,
    /// bochs-emu/Bochs#791). Default `None` (leaves not enumerated; guests
    /// PIT-calibrate the true tick rate) — deliberate divergence from the
    /// Bochs default `hardware`, which makes modern Linux trust the dumped
    /// multi-GHz TSC frequency and run all TSC-derived time `freq/ips` slow.
    pub cpuid_freq: CpuidFreq,
    /// How the RTC is seeded at power-up — Bochs `clock: time0` (config.cc
    /// BXPN_CLOCK_TIME0). Default `Local`, matching Bochs, so the guest RTC
    /// shows host local wall-clock time; `Utc` or a fixed timestamp are
    /// available for UTC guests / deterministic boots.
    pub rtc_time0: crate::iodev::cmos::RtcInitTime,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            memory: MemorySize::default(),
            memory_block_size: 128 * 1024,
            ips: Ips::BOCHS_DEFAULT,
            pci_enabled: true,
            pci_vga: false,
            cpu_params: BxParams::default(),
            sync_slowdown: false,
            sync_realtime: false,
            smp_quantum: 16,
            cpuid_freq: CpuidFreq::default(),
            rtc_time0: crate::iodev::cmos::RtcInitTime::default(),
        }
    }
}

#[cfg(feature = "std")]
const SLOWDOWN_QUANTUM_USEC: u64 = 1_000;
#[cfg(feature = "std")]
const SLOWDOWN_MAX_DELAY_USEC: u32 = 1_500;
#[cfg(feature = "std")]
const SLOWDOWN_REALTIME_QUANTUM_USEC: u64 = 1_000_000;

/// One data-TLB slot, as [`Emulator::get_dtlb_info`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DtlbInfo {
    /// Linear page frame the slot is tagged with.
    pub lpf: u64,
    /// Physical page frame it maps to.
    pub ppf: u64,
    /// Per-privilege permission bits Bochs calls `accessBits`.
    pub access_bits: u32,
    /// Byte offset of the backing page into guest RAM, or `None` when the page
    /// has no direct mapping.
    pub ram_offset: Option<usize>,
}

#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SlowdownAction {
    next_delay_usec: u32,
    sleep_one_quantum: bool,
    next_last_time_usec: u64,
}

#[cfg(feature = "std")]
#[derive(Debug)]
struct SlowdownTimerState {
    start_time: std::time::Instant,
    start_emulated_time_usec: u64,
    last_time_usec: u64,
    timer_handle: Option<usize>,
}

#[cfg(feature = "std")]
impl SlowdownTimerState {
    fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
            start_emulated_time_usec: 0,
            last_time_usec: 0,
            timer_handle: None,
        }
    }

    fn initialize(
        &mut self,
        timer_handle: usize,
        emulated_time_usec: u64,
        host_time: std::time::Instant,
    ) {
        self.start_time = host_time;
        self.start_emulated_time_usec = emulated_time_usec;
        self.last_time_usec = 0;
        self.timer_handle = Some(timer_handle);
    }

    fn decide(
        total_emulated_usec: u64,
        total_realtime_usec: u64,
        last_time_usec: u64,
    ) -> SlowdownAction {
        let want_time = last_time_usec.saturating_add(SLOWDOWN_QUANTUM_USEC);
        SlowdownAction {
            next_delay_usec: if total_realtime_usec > total_emulated_usec {
                SLOWDOWN_MAX_DELAY_USEC
            } else {
                SLOWDOWN_QUANTUM_USEC as u32
            },
            sleep_one_quantum: want_time
                > total_realtime_usec.saturating_add(SLOWDOWN_REALTIME_QUANTUM_USEC),
            next_last_time_usec: want_time.max(total_realtime_usec),
        }
    }

    fn handle_timer(
        &mut self,
        emulated_time_usec: u64,
        host_time: std::time::Instant,
    ) -> SlowdownAction {
        let total_emulated_usec = emulated_time_usec.saturating_sub(self.start_emulated_time_usec);
        let total_realtime_usec =
            u64::try_from(host_time.duration_since(self.start_time).as_micros())
                .unwrap_or(u64::MAX);
        let action = Self::decide(
            total_emulated_usec,
            total_realtime_usec,
            self.last_time_usec,
        );
        self.last_time_usec = action.next_last_time_usec;
        action
    }
}

#[cfg(feature = "std")]
impl Default for SlowdownTimerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Emulator instance containing all hardware components
///
/// This struct owns the CPU, Memory, Devices, and PC System, providing
/// a fully self-contained emulator instance with no global state.
///
/// # Thread Safety
///
/// Each `Emulator` instance is `Send` and can be moved to a different thread.
/// Multiple instances can run concurrently without any shared state.
///
/// # Example
///
/// A machine is assembled by [`MachineBuilder`] and arrives at its reset
/// vector; the only way to get one is to build one.
///
/// ```no_run
/// use rusty_box::emulator::{EmulatorConfig, MachineBuilder};
///
/// # fn main() -> rusty_box::Result<()> {
/// # let bios_data: &[u8] = &[];
/// let mut machine = MachineBuilder::new(EmulatorConfig::default())
///     .bios(bios_data)
///     .build()?;
/// let outcome = machine.step_batch(100_000)?;
/// println!("{} instructions, stopped: {:?}", outcome.executed, outcome.stop);
/// # Ok(())
/// # }
/// ```
///
/// The memory backing is intentionally not publicly replaceable:
///
/// ```compile_fail
/// use rusty_box::emulator::{EmulatorConfig, MachineBuilder};
///
/// let mut machine = MachineBuilder::new(EmulatorConfig::default()).build().unwrap();
/// let _ = &mut machine.memory;
/// ```
pub struct Emulator<T: Instrumentation = ()> {
    /// Every CPU this machine has, boot processor at index 0 — Bochs
    /// `bx_cpu_array`. Each stays at a stable address for its own cached host
    /// mappings.
    ///
    /// One field, one declaration, no `cfg` (doctrine R0). Which storage the
    /// alias resolves to is what varies with the build — heap-owned under
    /// `alloc`, borrowed from the caller without it — and that is a type, not
    /// a variant that exists in some builds and not others.
    ///
    /// A type PARAMETER here would be the more general shape, and is where
    /// this lands at Phase I when `Profile` picks the store alongside the
    /// device set. It buys nothing yet: no build needs two stores at once, and
    /// `T` appears only inside the store, so a parameter would need a
    /// `PhantomData` to justify itself.
    cpus: cpu_store::MachineCpus<T>,
    /// Memory subsystem
    pub(crate) memory: BxMemC,
    /// Device controller (I/O port handlers). Crate-private (doctrine R3):
    /// consumers reach device state through named machine methods, never by
    /// walking machine internals.
    pub(crate) devices: BxDevicesC,
    /// Device manager (actual hardware devices). Crate-private — see `devices`.
    pub(crate) device_manager: DeviceManager,
    /// PC system (timers, A20, etc.). Crate-private — see `devices`.
    pub(crate) pc_system: BxPcSystemC,
    /// Derived scheduler membership. These masks deliberately remain advisory
    /// until Phase 8's scan oracle makes them authoritative.
    runnable_mask: CpuMask,
    lapic_work_mask: CpuMask,
    /// Bochs SMP scheduler remainder from `executed %= BX_SMP_PROCESSORS`.
    smp_tick_remainder: u64,
    /// True when the last `run_cpu_batch` advanced `pc_system` internally.
    /// SMP batches tick at Bochs round boundaries so LAPIC/pc-system timers
    /// fire before the next virtual CPU slice; outer loops must not tick them
    /// a second time.
    batch_advanced_pc_system: bool,
    #[cfg(feature = "std")]
    slowdown_timer: SlowdownTimerState,
    /// Configuration
    config: EmulatorConfig,
    /// Whether the emulator has been initialized
    initialized: bool,
    /// A failed in-place v3 restore may have partially mutated guest state.
    snapshot_restore_failed: bool,
    /// GUI instance (optional, can be None for headless operation)
    #[cfg(feature = "alloc")]
    gui: Option<Box<dyn BxGui>>,
    /// Output file for the port-0xE9 debug console (std feature only). BIOS
    /// message ports 0x400-0x403/0x500-0x503 go to the log instead, exactly
    /// like Bochs biosdev.cc.
    #[cfg(feature = "std")]
    bios_output_file: Option<std::fs::File>,
    /// Exit addresses for emu_start.
    pub(crate) exit_set: ExitSet,
    /// Handle of the VGA vertical-retrace timer (Bochs vgacore.cc
    /// `vga_vtimer_id`). Re-armed whenever the retrace period changes.
    pub(crate) vga_vertical_timer_handle: Option<usize>,
    /// Vertical period currently programmed into that timer, so it is only
    /// re-armed when the guest actually changes the display timing.
    pub(crate) vga_vertical_period_usec: u32,
    /// Shared stop flag: when set to true by another thread (typically a GUI
    /// thread), `run_interactive`/`step_batch` loops exit. Crate-private
    /// (doctrine R3): external consumers share it through
    /// [`Emulator::set_stop_flag`] and read it through [`Emulator::stop_flag`],
    /// which present the same shape under both `alloc` settings (R0).
    #[cfg(feature = "alloc")]
    pub(crate) stop_flag: Arc<AtomicBool>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) stop_flag: AtomicBool,
    /// Why *this machine* last raised `stop_flag`. The flag is shared with
    /// whatever host thread holds a clone, so it can only ever be a bool;
    /// this says which of the causes behind it applies when the machine is the
    /// one that raised it. Read only while the flag is up, and reset whenever
    /// it is found down, so a host raise is never attributed to a guest.
    pub(crate) stop_cause: StopCause,
}

impl<'a, T: Instrumentation> Emulator<T> {
    /// How many processors this machine has, boot processor included.
    ///
    /// Fixed at construction from the configured topology; a guest cannot
    /// change it.
    pub fn cpu_count(&self) -> usize {
        self.cpus.count()
    }

    pub(crate) fn cpu_ref(&self, index: usize) -> &BxCpuC<T> {
        self.cpus.get(index)
    }

    pub(crate) fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<T> {
        self.cpus.get_mut(index)
    }

    /// Borrow one CPU together with the machine it executes against.
    ///
    /// This is the whole point of `ExecCtx`: the CPU, memory, devices, device
    /// models and PC system are separate fields, so a single destructuring
    /// hands out `&mut` to each simultaneously. Every path that needs more
    /// than one of them at once goes through here (doctrine R3), which is why
    /// the machine holds no pointer to any of its own parts.
    pub(crate) fn exec_ctx(&mut self, index: usize) -> crate::cpu::exec_ctx::ExecCtx<'_, T> {
        let Self {
            cpus,
            memory,
            devices,
            device_manager,
            pc_system,
            ..
        } = self;
        // `get_mut` borrows only the store, so memory, devices and the PC
        // system stay independently live — the disjointness `ExecCtx` rests on.
        crate::cpu::exec_ctx::ExecCtx::new(
            cpus.get_mut(index),
            memory,
            devices,
            device_manager,
            pc_system,
        )
    }


    #[cfg(feature = "std")]
    pub(crate) fn finish_snapshot_restore_v3(
        &mut self,
        live_bmdma: u16,
        live_pm: u16,
        live_sm: u16,
        live_vga: crate::iodev::vga::VgaSnapshotRestoreTarget,
        platform: crate::iodev::devices::PlatformSnapshotRestore,
        keyboard: crate::iodev::keyboard::KeyboardSnapshotRestore,
        cmos: crate::iodev::cmos::CmosSnapshotRestoreState,
        acpi: crate::iodev::acpi::AcpiSnapshotRestore,
        vga: crate::iodev::vga::VgaSnapshotRestoreTarget,
        pci: crate::iodev::pci_ide::PciIdeSnapshotTopology,
    ) -> std::io::Result<()> {
        self.device_manager
            .apply_snapshot_v3_restore(
                &mut self.devices,
                &mut self.memory,
                live_bmdma,
                live_pm,
                live_sm,
                live_vga,
                platform,
                pci,
                acpi,
                vga,
            )
            .map_err(|error| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
            })?;

        let sci_level = self
            .device_manager
            .acpi
            .post_restore_snapshot_v3(self.pc_system.time_ticks());
        self.device_manager.serial.after_restore_snapshot_v3()?;
        self.device_manager
            .vga
            .rebuild_snapshot_v3_derived_state()?;
        self.validate_restored_irq_levels(&keyboard, &cmos, sci_level)?;
        self.sync_restored_event_levels();
        self.rebuild_cpu_masks_from_scan();
        self.batch_advanced_pc_system = false;
        Ok(())
    }

    /// Re-anchor host pacing after a restore and validate the Slowdown owner
    /// against the live configuration.
    ///
    /// Host anchors (wall-clock `Instant`, accrued lead/lag) are deliberately
    /// never serialized — host time is not guest state — so a restore must
    /// restart lead/lag accumulation at zero from the restored virtual clock
    /// and the current wall clock (plan restore-hook step 8: host pause time
    /// is not charged). Cross-configuration restores are rejected like the
    /// section's IPS-mismatch precedent.
    #[cfg(feature = "std")]
    pub(crate) fn reanchor_slowdown_after_restore(&mut self) -> std::io::Result<()> {
        use std::io::{Error, ErrorKind};
        let restored_slot = self
            .pc_system
            .find_timer_slot_by_owner(TimerOwner::Slowdown);
        match (
            self.config.sync_slowdown,
            self.slowdown_timer.timer_handle,
            restored_slot,
        ) {
            (false, None, None) => Ok(()),
            (true, Some(handle), Some(slot)) if slot == handle => {
                self.pc_system
                    .validate_timer_handle_owner(handle, TimerOwner::Slowdown)?;
                let emulated_now = self.pc_system.time_usec();
                self.slowdown_timer
                    .initialize(handle, emulated_now, std::time::Instant::now());
                // A genuine snapshot always carries the one-shot armed or its
                // fire queued; rearm defensively if neither survived so
                // pacing resumes.
                if !self.pc_system.is_timer_active(handle)
                    && !self.pc_system.has_fired_owner(TimerOwner::Slowdown)
                {
                    self.pc_system
                        .activate_timer_usec(handle, SLOWDOWN_QUANTUM_USEC as u32, false)
                        .map_err(|error| {
                            Error::new(
                                ErrorKind::InvalidData,
                                format!("slowdown pacing rearm failed: {error:?}"),
                            )
                        })?;
                }
                Ok(())
            }
            _ => Err(Error::new(
                ErrorKind::InvalidData,
                "snapshot slowdown pacing owner does not match live configuration",
            )),
        }
    }

    /// Reject a snapshot whose device sections disagree with the restored PIC
    /// input lines.
    ///
    /// Bochs pic.cc `bx_pic_c::register_state` restores `IRQ_in` from the
    /// PIC's own saved state and no device re-raises its line on restore; a
    /// correct quiesced save therefore always agrees. Re-driving a level here
    /// would emit artificial IOAPIC edges, so a disagreement is corrupt input
    /// and poisons the restore. Checks are skipped while a serialized
    /// in-flight edge latch says a transition is still queued for the first
    /// post-restore boundary.
    #[cfg(feature = "std")]
    fn validate_restored_irq_levels(
        &self,
        keyboard: &crate::iodev::keyboard::KeyboardSnapshotRestore,
        cmos: &crate::iodev::cmos::CmosSnapshotRestoreState,
        sci_level: bool,
    ) -> std::io::Result<()> {
        fn mismatch(what: &'static str) -> std::io::Error {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("restored {what} level disagrees with restored PIC line"),
            )
        }
        let pic = &self.device_manager.pic;
        let kbd = &self.device_manager.keyboard.kbd_controller;

        if !kbd.irq1_requested && pic.irq_line_level(1) != keyboard.irq1_level {
            return Err(mismatch("keyboard IRQ1"));
        }
        if !kbd.irq12_requested && pic.irq_line_level(12) != keyboard.irq12_level {
            return Err(mismatch("mouse IRQ12"));
        }

        let cmos_live = &self.device_manager.cmos;
        if !cmos_live.irq8_pending
            && !cmos_live.irq8_lower_pending
            && pic.irq_line_level(8) != cmos.irq8_level
        {
            return Err(mismatch("RTC IRQ8"));
        }

        // `pm_update_sci` recomputes deterministically from restored
        // PMSTS/PMEN state, so a genuine snapshot always agrees.
        if pic.irq_line_level(9) != sci_level {
            return Err(mismatch("ACPI SCI IRQ9"));
        }

        for (channel, irq) in [(0usize, 14u8), (1, 15)] {
            // An in-flight seek (armed "HD/CD seek" timer or an undrained arm
            // latch) will raise/complete the IRQ when its deadline fires —
            // the line level is legitimately transitional then.
            let seek_in_flight = (0..2).any(|device| {
                self.device_manager.ide.drives.pending_seek_arm_usec[channel][device].is_some()
                    || self.device_manager.ide.drives.seek_timer_handles[channel][device]
                        .is_some_and(|handle| self.pc_system.is_timer_active(handle))
            });
            if seek_in_flight {
                continue;
            }
            if pic.irq_line_level(irq) != self.device_manager.ide.drives.get_irq_level(channel) {
                return Err(mismatch("ATA IRQ"));
            }
        }

        let serial = &self.device_manager.serial;
        for port in 0..serial.configured_port_count() {
            if serial.has_pending_irq_transition(port) {
                continue;
            }
            if let Some((irq, level)) = serial.restored_irq_line(port) {
                if pic.irq_line_level(irq) != level {
                    return Err(mismatch("serial IRQ"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl Emulator<()> {
    /// Construct an uninitialised machine with no instrumentation.
    ///
    /// In-crate shorthand for the untraced factory; every machine that leaves
    /// this crate is assembled by [`MachineBuilder`] instead.
    pub(crate) fn new(config: EmulatorConfig) -> Result<Box<Self>> {
        Self::with_tracer_factory(config, || ())
    }
}

#[cfg(feature = "alloc")]
impl<'a, T: Instrumentation> Emulator<T> {
    /// Construct an uninitialised machine whose sole processor carries
    /// `tracer`. One tracer instance cannot be shared, so this shape is
    /// uniprocessor; `MachineBuilder` is what enforces that.
    pub(crate) fn with_tracer(config: EmulatorConfig, tracer: T) -> Result<Box<Self>> {
        let requested = config.cpu_params.cpu_count();
        if requested > 1 {
            return Err(BuildError::InstrumentedSmp { requested }.into());
        }
        let topology = config.cpu_params.cpu_topology();
        let mut cpu = BxCpuBuilder::new().build_with_tracer(tracer)?;
        cpu.configure_smp(0, topology);
        cpu.set_smp_quantum(config.smp_quantum);
        cpu.set_cpuid_freq(config.cpuid_freq, config.ips.per_second());
        Self::new_from_parts(config, cpu_store::OwnedCpus::new(alloc::vec![cpu]))
    }

    /// Construct an uninitialised machine, minting a fresh tracer for every
    /// processor. The tracer type `T` is baked in at construction and cannot
    /// be changed; all tracer dispatch is inlined.
    pub(crate) fn with_tracer_factory(
        config: EmulatorConfig,
        make_tracer: fn() -> T,
    ) -> Result<Box<Self>> {
        Self::new_inner(config, move || {
            Ok(BxCpuBuilder::new().build_with_tracer(make_tracer())?)
        })
    }

    fn new_inner<F>(config: EmulatorConfig, mut build_cpu: F) -> Result<Box<Self>>
    where
        F: FnMut() -> Result<alloc::boxed::Box<BxCpuC<T>>>,
    {
        let topology = config.cpu_params.cpu_topology();
        let cpu_count = config.cpu_params.cpu_count();
        let mut cpu = build_cpu()?;
        cpu.configure_smp(0, topology);
        cpu.set_smp_quantum(config.smp_quantum);
        cpu.set_cpuid_freq(config.cpuid_freq, config.ips.per_second());

        let mut cpus = Vec::with_capacity(cpu_count as usize);
        cpus.push(cpu);
        for cpu_id in 1..cpu_count {
            let mut ap_cpu = build_cpu()?;
            ap_cpu.configure_smp(cpu_id, topology);
            ap_cpu.set_smp_quantum(config.smp_quantum);
            ap_cpu.set_cpuid_freq(config.cpuid_freq, config.ips.per_second());
            cpus.push(ap_cpu);
        }
        Self::new_from_parts(config, cpu_store::OwnedCpus::new(cpus))
    }

    fn new_from_parts(
        config: EmulatorConfig,
        cpus: cpu_store::OwnedCpus<T>,
    ) -> Result<Box<Self>> {
        let pc_system = BxPcSystemC::new();
        let mem_stub = BxMemoryStubC::create_and_init(
            config.memory.guest_bytes(),
            config.memory.host_bytes(),
            config.memory_block_size,
        )?;
        let memory = BxMemC::new(mem_stub, config.pci_enabled);
        let devices = BxDevicesC::new();
        let device_manager = DeviceManager::new();

        // Emulator contains large fixed arrays. Allocate zeroed on heap
        // then write fields to avoid stack overflow on UEFI (128KB stack).
        let layout = alloc::alloc::Layout::new::<Self>();
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut Self;
        if ptr.is_null() {
            return Err(MemoryError::UnableToAllocateGuestMemory(layout.size()).into());
        }
        unsafe {
            core::ptr::addr_of_mut!((*ptr).cpus).write(cpus);
            core::ptr::addr_of_mut!((*ptr).memory).write(memory);
            core::ptr::addr_of_mut!((*ptr).devices).write(devices);
            core::ptr::addr_of_mut!((*ptr).device_manager).write(device_manager);
            core::ptr::addr_of_mut!((*ptr).pc_system).write(pc_system);
            core::ptr::addr_of_mut!((*ptr).runnable_mask).write(CpuMask::default());
            core::ptr::addr_of_mut!((*ptr).lapic_work_mask).write(CpuMask::default());
            core::ptr::addr_of_mut!((*ptr).smp_tick_remainder).write(0);
            core::ptr::addr_of_mut!((*ptr).batch_advanced_pc_system).write(false);
            #[cfg(feature = "std")]
            core::ptr::addr_of_mut!((*ptr).slowdown_timer).write(SlowdownTimerState::new());
            core::ptr::addr_of_mut!((*ptr).config).write(config);
            core::ptr::addr_of_mut!((*ptr).initialized).write(false);
            core::ptr::addr_of_mut!((*ptr).snapshot_restore_failed).write(false);
            core::ptr::addr_of_mut!((*ptr).gui).write(None);
            #[cfg(feature = "std")]
            core::ptr::addr_of_mut!((*ptr).bios_output_file).write(None);
            core::ptr::addr_of_mut!((*ptr).exit_set).write(ExitSet::new());
            core::ptr::addr_of_mut!((*ptr).vga_vertical_timer_handle).write(None);
            core::ptr::addr_of_mut!((*ptr).vga_vertical_period_usec).write(0);
            core::ptr::addr_of_mut!((*ptr).stop_flag).write(Arc::new(AtomicBool::new(false)));
            core::ptr::addr_of_mut!((*ptr).stop_cause).write(StopCause::default());
            Ok(alloc::boxed::Box::from_raw(ptr))
        }
    }
}

impl<'a, T: Instrumentation> Emulator<T> {
    #[cfg(not(feature = "alloc"))]
    /// Construct a machine inside caller-provided storage.
    ///
    /// A no-alloc host owns the three big allocations, so it supplies this
    /// machine's storage, its processors and an already-initialised
    /// `BxMemoryStubC` (typically over a firmware-provided buffer).
    ///
    /// `cpus` is the machine's whole CPU set, boot processor at index 0 — the
    /// same order the `alloc` store uses. It is a slice of exclusive borrows
    /// rather than a slice of CPUs so a no-alloc host can place each CPU
    /// wherever it likes; nothing requires them to be contiguous.
    pub(crate) fn init_at(
        storage: &'a mut core::mem::MaybeUninit<Self>,
        cpus: &'static mut [&'static mut BxCpuC<T>],
        mem_stub: BxMemoryStubC,
        config: EmulatorConfig,
    ) -> Result<&'a mut Self> {
        let topology = config.cpu_params.cpu_topology();
        let configured_cpu_count = config.cpu_params.cpu_count() as usize;
        if cpus.len() < configured_cpu_count {
            return Err(crate::cpu::CpuError::UnsupportedCpuOperation {
                operation: "no-alloc SMP requires caller-provided CPU storage",
            }
            .into());
        }

        let memory = BxMemC::new_from_stub(mem_stub, config.pci_enabled);
        let devices = BxDevicesC::new();
        let device_manager = DeviceManager::new();
        let pc_system = BxPcSystemC::new();
        for (index, slot) in cpus.iter_mut().take(configured_cpu_count).enumerate() {
            slot.configure_smp(index as u32, topology);
            slot.set_smp_quantum(config.smp_quantum);
            slot.set_cpuid_freq(config.cpuid_freq, config.ips.per_second());
        }
        let cpus = cpu_store::BorrowedCpus::new(&mut cpus[..configured_cpu_count]);
        let ptr = storage.as_mut_ptr();
        // SAFETY: `storage` is an exclusive borrow of an allocation sized and
        // aligned for `Self`, and every field below is written before the
        // reference is created, so no caller can observe uninitialised state.
        // The descriptor sidecars are 40 KiB each, which is why the fields are
        // written in place rather than through a `Self { .. }` temporary that
        // would have to be built on the stack and moved.
        unsafe {
            core::ptr::addr_of_mut!((*ptr).cpus).write(cpus);
            core::ptr::addr_of_mut!((*ptr).memory).write(memory);
            core::ptr::addr_of_mut!((*ptr).devices).write(devices);
            core::ptr::addr_of_mut!((*ptr).device_manager).write(device_manager);
            core::ptr::addr_of_mut!((*ptr).pc_system).write(pc_system);
            core::ptr::addr_of_mut!((*ptr).runnable_mask).write(CpuMask::default());
            core::ptr::addr_of_mut!((*ptr).lapic_work_mask).write(CpuMask::default());
            core::ptr::addr_of_mut!((*ptr).smp_tick_remainder).write(0);
            core::ptr::addr_of_mut!((*ptr).batch_advanced_pc_system).write(false);
            #[cfg(feature = "std")]
            core::ptr::addr_of_mut!((*ptr).slowdown_timer).write(SlowdownTimerState::new());
            core::ptr::addr_of_mut!((*ptr).config).write(config);
            core::ptr::addr_of_mut!((*ptr).initialized).write(false);
            core::ptr::addr_of_mut!((*ptr).snapshot_restore_failed).write(false);
            core::ptr::addr_of_mut!((*ptr).exit_set).write(ExitSet::new());
            core::ptr::addr_of_mut!((*ptr).vga_vertical_timer_handle).write(None);
            core::ptr::addr_of_mut!((*ptr).vga_vertical_period_usec).write(0);
            core::ptr::addr_of_mut!((*ptr).stop_flag).write(AtomicBool::new(false));
            core::ptr::addr_of_mut!((*ptr).stop_cause).write(StopCause::default());
            Ok(&mut *ptr)
        }
    }

    fn configure_pci_devices(&mut self) {
        self.devices.set_pci_enabled(self.config.pci_enabled);
        let ramsize_mb = (self.config.memory.guest_bytes() / (1024 * 1024)) as u32;
        self.device_manager.pci_bridge.init_dram(ramsize_mb);
        if self.config.pci_enabled && self.config.pci_vga {
            self.device_manager.vga.enable_pci();
            tracing::info!("VGA registered as PCI device (1234:1111, class 0300)");
        }
        tracing::trace!("PCI bridge DRAM initialized for {}MB", ramsize_mb);
    }

    /// Bring the hardware up with no firmware and no media — the shape a test
    /// wants when it drives the machine through the register-level API rather
    /// than booting it.
    ///
    /// Firmware belongs between the two halves (Bochs main.cc loads the BIOS
    /// after memory init and before CPU init), which is why
    /// [`MachineBuilder`] and not this method is what boots a machine.
    #[cfg(all(feature = "alloc", test))]
    pub(crate) fn initialize(&mut self) -> Result<()> {
        self.init_memory_and_pc_system()?;
        self.init_cpu_and_devices()
    }

    /// Initialize memory and PC system (Step 1-2 of initialization)
    ///
    /// This is the first part of the initialization sequence from Bochs main.cc:
    /// 1. PC system initialization (timers, IPS) - line 1201
    /// 2. Memory initialization - line 1312
    ///
    /// After this, call `load_bios()` and `load_optional_rom()`, then `init_cpu_and_devices()`.
    /// This matches the original Bochs sequence: Memory init → Load BIOS → CPU init → Device init.
    #[cfg(feature = "alloc")]
    pub(crate) fn init_memory_and_pc_system(&mut self) -> Result<()> {
        if self.initialized {
            tracing::trace!("Emulator already initialized");
            return Ok(());
        }

        tracing::debug!("Initializing hardware...");

        // Step 1: Initialize PC system with IPS (line 1201)
        self.pc_system.initialize(self.config.ips.per_second());
        self.devices.set_timer_ips(self.config.ips.per_second_u64());
        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        tracing::trace!("PC system initialized with {} IPS", self.config.ips.per_second());

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
        self.invalidate_all_cpu_host_mappings();
        self.memory.init_memory(
            self.config.memory.guest_bytes(),
            self.config.memory.host_bytes(),
            self.config.memory_block_size,
        )?;

        // Sync A20 mask from PC system (after memory init, matching original)
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        tracing::trace!("Memory initialized and A20 mask synced");

        Ok(())
    }

    /// Initialize PC system timers and sync A20 mask.
    ///
    /// The no-alloc machine's memory arrives already initialised inside a
    /// caller-built stub, so this is the half of
    /// `init_memory_and_pc_system` that still has work to do there.
    #[cfg(not(feature = "alloc"))]
    pub(crate) fn init_pc_system(&mut self) {
        self.pc_system.initialize(self.config.ips.per_second());
        self.smp_tick_remainder = 0;
        self.memory.set_a20_mask(self.pc_system.a20_mask());
    }

    /// Initialize CPU and devices (Step 6-11 of initialization)
    ///
    /// This is the second part of the initialization sequence from Bochs main.cc:
    /// 6. CPU initialization - line 1337
    /// 7. CPU sanity checks - line 1338
    /// 8. CPU register state - line 1339
    /// 9. Device initialization - line 1353
    /// 10. PC system register state - line 1356
    /// 11. Device register state - line 1357
    ///
    /// Call this AFTER `init_memory_and_pc_system()` and `load_bios()`.
    pub(crate) fn init_cpu_and_devices(&mut self) -> Result<()> {
        // The no-alloc construction path (`init_at`) has no separate
        // `init_memory_and_pc_system` step, so make this initializer
        // self-sufficient: without it a no-alloc machine ran every device
        // timer conversion against the default `ips = 1`. Re-running it in
        // the alloc flow is harmless — no timers are registered until
        // `register_timer_owners` below and no virtual time has advanced.
        self.pc_system.initialize(self.config.ips.per_second());
        self.devices.set_timer_ips(self.config.ips.per_second_u64());
        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;

        let cpu_params = self.config.cpu_params.clone();
        for cpu_index in 0..self.cpu_count() {
            self.cpu_mut_at(cpu_index).initialize(cpu_params.clone())?;
        }
        tracing::trace!("CPUs initialized");

        // Step 7: CPU sanity checks (line 1338) - separate call to match original
        for cpu_index in 0..self.cpu_count() {
            self.cpu_mut_at(cpu_index).sanity_checks()?;
        }
        tracing::trace!("CPU sanity checks passed");

        // Step 8: Register CPU state (line 1339)
        for cpu_index in 0..self.cpu_count() {
            self.cpu_ref(cpu_index).register_state();
        }
        tracing::trace!("CPU state registered");

        // Note: BX_INSTR_INITIALIZE(0) at line 1340 is instrumentation initialization
        // This is optional and not yet implemented in Rust version

        // Step 9: Initialize devices (line 1353)
        self.devices.init(&mut self.memory)?;

        // Bochs clock:time0 — apply the RTC power-up seed source (local / utc /
        // fixed) from config. The CMOS was seeded at construction with the Utc
        // default; re-seed it here so the guest's RTC matches the configuration
        // before any device reset or the BIOS reads it.
        self.device_manager.cmos.set_time0(self.config.rtc_time0);

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;

        self.configure_pci_devices();
        // Initialize fw_cfg device and ACPI CPU/APIC tables.
        {
            let ram_size = self.config.memory.guest_bytes() as u64;
            let cpu_count = self.config.cpu_params.cpu_count();
            self.device_manager.ioapic.set_id(cpu_count);
            self.device_manager.fw_cfg.init(ram_size, cpu_count);
            #[cfg(feature = "alloc")]
            {
                let acpi = AcpiTableGenerator::generate(ram_size, cpu_count);
                self.device_manager.fw_cfg.add_acpi_tables(
                    acpi.tables_blob(),
                    acpi.rsdp_blob(),
                    acpi.loader_blob(),
                );
            }
        }
        tracing::debug!("Device initialization complete");

        self.register_timer_owners()?;

        // Note: SIM->opt_plugin_ctrl("*", 0) at line 1355 unloads unused optional plugins
        // This is optional plugin management, not yet implemented in Rust version

        // Step 10: PC system register state (line 1356)
        self.pc_system.register_state();

        // Step 11: Device register state (line 1357)
        self.devices.register_state()?;
        tracing::trace!("State registered");

        // Note: bx_set_log_actions_by_device(1) at line 1359 sets up logging per device
        // This is only called if not restoring state, and is optional logging setup

        self.rebuild_cpu_masks_from_scan();
        self.snapshot_restore_failed = false;
        self.initialized = true;
        tracing::debug!("Emulator initialization complete");

        // Note: Steps 12-14 (Reset, GUI signal handlers, Start timers) are done via:
        // - reset() method (called after BIOS loading)
        // - init_gui() method (calls init_signal_handlers)
        // - reset() also calls start_timers()

        Ok(())
    }

    #[cfg(feature = "alloc")]
    /// Install the display front end.
    ///
    /// Bochs main.cc `load_and_init_display_lib`, which runs before
    /// `bx_init_hardware`; `MachineBuilder` preserves that ordering.
    pub(crate) fn set_boxed_gui(&mut self, gui: Box<dyn BxGui>) {
        self.gui = Some(gui);
        tracing::debug!("GUI set");
    }

    #[cfg(feature = "alloc")]
    /// Initialize the GUI
    ///
    /// Based on bx_init_hardware() GUI initialization in main.cc
    /// This calls specific_init() to set up the GUI, but signal handlers are
    /// initialized separately via init_gui_signal_handlers() after reset.
    pub(crate) fn init_gui(&mut self, argc: i32, argv: &[&str]) -> Result<()> {
        if let Some(ref mut gui) = self.gui {
            gui.specific_init(argc, argv, 32); // BX_HEADER_BAR_Y = 32
            gui.update_drive_status_buttons();

            // Connect keyboard callback if GUI supports it
            self.connect_keyboard_callback();

            tracing::debug!("GUI initialized (signal handlers will be set up after reset)");
        } else {
            tracing::trace!("No GUI set, running headless");
        }
        Ok(())
    }

    #[cfg(feature = "alloc")]
    /// Connect keyboard callback from GUI to keyboard device
    /// (No-op now - we use queue-based approach instead)
    fn connect_keyboard_callback(&mut self) {
        // Keyboard input is now handled via get_pending_scancodes() in the event loop
    }

    #[cfg(feature = "alloc")]
    /// Get mutable reference to GUI (if set)
    pub fn gui_mut(&mut self) -> Option<&mut (dyn BxGui + 'static)> {
        self.gui.as_deref_mut()
    }

    /// Get an immutable reference to the stable BSP CPU allocation.
    #[inline]
    pub fn cpu(&self) -> &BxCpuC<T> {
        self.cpu_ref(0)
    }

    #[cfg(feature = "alloc")]
    /// Get reference to GUI (if set)
    pub fn gui(&self) -> Option<&(dyn BxGui + 'static)> {
        self.gui.as_deref()
    }

    /// Mutably access the BSP CPU for crate-internal emulator operations.
    ///
    /// This is crate-visible so the public API can expose targeted operations
    /// without allowing safe replacement of the CPU storage itself.
    #[inline]
    pub(crate) fn cpu_mut(&mut self) -> &mut BxCpuC<T> {
        self.cpu_mut_at(0)
    }

    /// Mutably access the boot CPU without moving it.
    ///
    /// Prefer the targeted safe `Emulator` operations whenever one exists.
    /// This escape hatch is for external integrations that need arbitrary CPU
    /// state mutation.
    ///
    /// # Safety
    ///
    /// The caller must not move, replace or swap the CPU, nor retain
    /// references or raw pointers taken from it beyond the borrow this returns.
    ///
    /// The invariant owner is the TLB: its entries cache page *numbers*
    /// measured against this machine's memory allocation, and `cpu/access.rs`
    /// reconstructs a raw host pointer from each one — `host.add(offset)`,
    /// dereferenced with no bounds check — using a base that `ExecCtx` derives
    /// from the machine the CPU belongs to. Giving this CPU to another machine
    /// reinterprets those numbers against a different, possibly smaller
    /// allocation, so the next memory access dereferences out of bounds.
    /// Nothing in the type system stops `core::mem::swap` on two of these
    /// borrows, which is why the function carries the obligation instead.
    ///
    /// ```compile_fail
    /// use rusty_box::cpu::core_i7_skylake::Corei7SkylakeX;
    /// use rusty_box::emulator::{Emulator, EmulatorConfig};
    ///
    /// let mut first = Emulator::new(EmulatorConfig::default()).unwrap();
    /// let mut second = Emulator::new(EmulatorConfig::default()).unwrap();
    /// // Safe code cannot obtain mutable CPU storage to swap it.
    /// core::mem::swap(first.cpu_mut(), second.cpu_mut());
    /// ```
    #[inline]
    pub unsafe fn cpu_mut_unchecked(&mut self) -> &mut BxCpuC<T> {
        self.cpu_mut_at(0)
    }

    /// Load a BIOS ROM image
    ///
    /// # Arguments
    /// * `bios_data` - Raw BIOS ROM data
    /// * `address` - Load address (typically 0xfffe0000 for 128KB BIOS)
    pub(crate) fn load_bios(&mut self, bios_data: &[u8], address: u64) -> Result<()> {
        self.memory.load_ROM(bios_data, address, 0)?;
        tracing::debug!("Loaded BIOS ({} bytes) at {:#x}", bios_data.len(), address);
        Ok(())
    }

    /// Load an optional ROM image (VGA BIOS, expansion ROMs, etc.)
    ///
    /// # Arguments
    /// * `rom_data` - Raw ROM data
    /// * `address` - Load address (must be in 0xC0000-0xFFFFF range)
    pub(crate) fn load_optional_rom(&mut self, rom_data: &[u8], address: u64) -> Result<()> {
        self.memory.load_ROM(rom_data, address, 2)?;
        tracing::debug!(
            "Loaded optional ROM ({} bytes) at {:#x}",
            rom_data.len(),
            address
        );
        Ok(())
    }

    /// Load an optional RAM image
    ///
    /// Based on `BX_MEM(0)->load_RAM()` in Bochs main.cc
    ///
    /// # Arguments
    /// * `ram_data` - Raw RAM image data
    /// * `address` - Load address in physical memory
    #[cfg(test)]
    pub(crate) fn load_ram(&mut self, ram_data: &[u8], address: u64) -> Result<()> {
        // Stable CPU pin storage outlives this exclusive memory borrow.
        self.memory.load_RAM(ram_data, address)?;
        tracing::debug!(
            "Loaded RAM image ({} bytes) at {:#x}",
            ram_data.len(),
            address
        );
        Ok(())
    }

    /// Perform a system reset
    ///
    /// This corresponds to `bx_pc_system.Reset()` in Bochs.
    ///
    /// # Arguments
    /// * `reset_type` - Type of reset (Hardware or Software)
    pub fn reset(&mut self, reset_type: ResetReason) -> Result<()> {
        let recovering_failed_snapshot = self.snapshot_restore_failed;
        tracing::debug!("Emulator reset ({:?})", reset_type);
        self.devices.discard_scheduler_boundary_work();

        // Reset PC system (enables A20)
        self.pc_system.reset(reset_type);

        // Sync A20 mask to memory
        self.memory.set_a20_mask(self.pc_system.a20_mask());

        // Reset all CPUs. CPU 0 is BSP; APs enter WAIT_FOR_SIPI in BxCpuC::reset.
        for cpu_index in 0..self.cpu_count() {
            self.cpu_mut_at(cpu_index).reset(reset_type);
        }

        for cpu_index in 0..self.cpu_count() {
            let timer_handle = {
                let cpu = self.cpu_mut_at(cpu_index);
                let timer_handle = cpu.lapic.timer_handle;
                cpu.lapic.timer_deactivate_request = false;
                cpu.lapic.timer_activate_request = None;
                cpu.lapic.timer_fired = false;
                timer_handle
            };
            if let Some(handle) = timer_handle {
                if let Err(e) = self.pc_system.deactivate_timer(handle) {
                    tracing::error!(
                        "CPU {cpu_index} LAPIC timer deactivate on reset (handle {handle}) failed: {e:?}"
                    );
                }
            }
        }

        // Reset devices (only on hardware reset)
        // Matches original: DEV_reset_devices(type) at pc_system.cc
        // which calls bx_devices_c::reset() at devices.cc
        if matches!(reset_type, ResetReason::Hardware) {
            // Original bx_devices_c::reset() does (in order):
            // 1. Clear PCI confAddr if PCI enabled (line 402) - done in devices.reset()
            // 2. mem->disable_smram() (line 405) - disable SMRAM
            // 3. bx_reset_plugins(type) (line 406) - reset all device plugins
            // 4. release_keys() (line 407) - release keyboard keys
            // 5. paste.stop = 1 (line 409) - stop paste buffer

            // Step 1: Clear PCI confAddr (done in devices.reset())
            self.devices.reset(reset_type)?;

            // Step 2: Disable SMRAM (matches original line 405: mem->disable_smram())
            self.memory.disable_smram();

            // Reset the machine-wide SMC write-stamp table (Bochs
            // pageWriteStampTable.resetWriteStamps on hardware reset; every
            // cpu's icache is flushed by the cpu resets below, so no stale
            // trace can outlive its stamps).
            self.memory.smc_reset_stamps();

            // Step 3: Reset all device plugins (matches original line 406: bx_reset_plugins())
            // This resets all devices: PIC, PIT, CMOS, DMA, Keyboard, HardDrive, VGA
            self.device_manager.reset(reset_type)?;
            self.rearm_device_timers_after_hardware_reset();

            // Note: release_keys() at line 407 and paste.stop at line 409 not yet implemented
        }

        // Reset always enables A20. Discard requests made before this reset
        // and synchronize only the A20 mirrors; software reset must leave all
        // unrelated controller/device state intact.
        if matches!(reset_type, ResetReason::Hardware) {
            self.device_manager.port92 = SystemControlPort::new();
        } else {
            self.device_manager.port92.reset_request = None;
        }
        let a20_enabled = self.pc_system.get_enable_a20();
        self.device_manager.port92.a20_gate = a20_enabled;
        self.device_manager.port92.a20_change_pending = false;
        self.device_manager.keyboard.a20_enabled = a20_enabled;
        self.device_manager.keyboard.a20_change_pending = false;
        self.device_manager.keyboard.reset_requested = None;

        // Note: start_timers() is called separately after GUI signal handlers
        // to match original Bochs order: reset -> init_signal_handlers -> start_timers

        self.rebuild_cpu_masks_from_scan();
        if recovering_failed_snapshot {
            self.initialized = true;
        }
        self.snapshot_restore_failed = false;
        Ok(())
    }

    #[cfg(feature = "alloc")]
    /// Initialize GUI signal handlers
    ///
    /// This should be called after reset() and before start_timers() to match
    /// original Bochs sequence (line 1383).
    pub(crate) fn init_gui_signal_handlers(&mut self) {
        if let Some(ref mut gui) = self.gui {
            gui.init_signal_handlers();
            tracing::trace!("GUI signal handlers initialized");
        }
    }

    /// Arm the timer wheel, the last step of Bochs main.cc's hardware bring-up.
    pub(crate) fn start_timers(&mut self) {
        self.pc_system.start_timers();
        tracing::trace!("Timers started");
    }

    /// Prepare for execution (start timers and log)
    ///
    /// Call this before entering the CPU loop.
    pub fn prepare_run(&mut self) {
        tracing::trace!("Starting CPU execution at RIP={:#x}", self.cpu_ref(0).rip());

        // Initialize PIT icount sync so PIT counter reads advance with CPU time.
        // This is critical for kernel PIT-polling calibration loops (e.g., Alpine Linux).
        let ips = self.config.ips.per_second_u64();
        if ips > 0 {
            // The PIT/ACPI absolute time cursor lives in the system-tick
            // domain — the same clock the port-I/O read paths pass via
            // `system_ticks()`. At cold boot this equals icount (both 0);
            // after HLT fast-forwards or fast-REP surpluses only the tick
            // clock is correct.
            let now_ticks = self.pc_system.time_ticks();
            self.device_manager.pit.init_icount_sync(now_ticks, ips);
            self.device_manager.acpi.init_icount_sync(now_ticks, ips);
            // Bochs `clock: sync=realtime` only when configured (pit.cc reads
            // bx_virt_timer with is_realtime from the clock option); with the
            // default sync=none the timers stay on emulated (icount) time.
            #[cfg(feature = "std")]
            if self.config.sync_realtime {
                self.device_manager.pit.enable_realtime_sync();
                self.device_manager.acpi.enable_realtime_sync();
            }
        }

        // Initialize VGA icount-based timing for retrace computation.
        {
            let ips = self.config.ips.per_second_u64();
            self.device_manager.vga.set_icount_sync(ips);
        }

        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        self.start_timers();
    }

    /// Get current instruction pointer
    pub fn rip(&self) -> u64 {
        self.cpu_ref(0).rip()
    }

    #[cfg(feature = "alloc")]
    /// Read up to `len` physical-RAM bytes for diagnostics.
    ///
    /// The result is intentionally a requested-size copy: guest RAM can be
    /// block-backed and swapped, so it is never exposed as a borrowed slice.
    pub fn peek_ram_at(&mut self, addr: usize, len: usize) -> alloc::vec::Vec<u8> {
        let mut bytes = alloc::vec![0; len];
        // Stable emulator pin storage outlives the exclusive memory borrow.
        let copied = self.memory.read_ram(addr as u64, &mut bytes).unwrap_or(0);
        bytes.truncate(copied);
        bytes
    }

    // Only the debug-assertions Alpine diagnostic dump consumes this.
    #[cfg(all(feature = "std", debug_assertions))]
    #[inline]
    fn read_physical_u64_or_zero(&mut self, addr: u64) -> u64 {
        let bytes = self.peek_ram_at(addr as usize, 8);
        bytes
            .as_slice()
            .try_into()
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    /// Read-only access to this emulator's configuration.
    pub fn config_ref(&self) -> &EmulatorConfig {
        &self.config
    }

    #[cfg(feature = "std")]
    pub(crate) fn mark_snapshot_restore_failed(&mut self) {
        self.initialized = false;
        self.snapshot_restore_failed = true;
    }

    /// Get the current system tick count
    pub fn ticks(&self) -> u64 {
        self.pc_system.time_ticks()
    }

    /// Apply an A20 transition and invalidate every CPU translation view.
    fn apply_a20_gate(&mut self, enabled: bool) -> bool {
        if enabled == self.pc_system.get_enable_a20() {
            return false;
        }
        self.pc_system.set_enable_a20(enabled);
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        true
    }

    /// Sync A20 state from system control port to PC system and memory.
    pub fn sync_a20_state(&mut self) {
        if self.apply_a20_gate(self.device_manager.port92.a20_gate) {
            self.invalidate_all_cpu_host_mappings();
        }
    }

    /// Queue Port 92 A20/reset work through the central machine boundary.
    /// Returns whether a reset was applied at this boundary.
    pub fn write_port_92h(&mut self, value: u8) -> bool {
        self.device_manager.port92.write(value);
        let a20_changed = self.device_manager.port92.a20_change_pending;
        let reset_requested = self.device_manager.port92.reset_request.is_some();
        if a20_changed || reset_requested {
            match self.service_scheduler_boundary(0) {
                Ok(reset_applied) => return reset_applied,
                Err(error) => {
                    tracing::error!("Port 92 scheduler boundary failed: {error:?}");
                }
            }
        }
        reset_requested
    }

    /// Read Port 92h value
    pub fn read_port_92h(&self) -> u8 {
        self.device_manager.port92.read()
    }

    /// Set the output file for the port-0xE9 debug console (requires std
    /// feature). BIOS message ports (0x400-0x403/0x500-0x503) are routed to
    /// the log per Bochs biosdev.cc and never appear in this stream.
    ///
    /// When set, port-0xE9 output will be written to this file instead of stdout.
    #[cfg(feature = "std")]
    pub fn set_bios_output_file(&mut self, file: std::fs::File) {
        self.bios_output_file = Some(file);
    }

    /// Attach a hard disk image (requires std feature)
    ///
    /// # Arguments
    /// * `channel` - ATA channel (0=primary, 1=secondary)
    /// * `drive` - Drive number (0=master, 1=slave)
    /// * `path` - Path to the disk image file
    /// * `cylinders` - Number of cylinders
    /// * `heads` - Number of heads
    /// * `spt` - Sectors per track
    #[cfg(feature = "std")]
    pub(crate) fn attach_disk(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
        cylinders: u32,
        heads: u8,
        spt: u8,
    ) -> std::io::Result<()> {
        self.device_manager
            .ide
            .drives
            .attach_disk(channel, drive, path, cylinders, heads, spt)
    }

    /// Attach a CD-ROM ISO image to a channel/drive (requires std feature)
    #[cfg(feature = "std")]
    pub(crate) fn attach_cdrom(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
    ) -> std::io::Result<()> {
        self.device_manager
            .ide
            .drives
            .attach_cdrom_image(channel, drive, path)
    }

    /// Configure CMOS memory size from total RAM bytes.
    /// This is the preferred method — it matches Bochs devices.cc.
    pub(crate) fn configure_memory_in_cmos_from_config(&mut self) {
        self.device_manager
            .cmos
            .set_memory_size_from_bytes(self.config.memory.guest_bytes() as u64);
    }

    /// Configure full CMOS hard drive geometry (matching Bochs harddrv.cc)
    pub(crate) fn configure_disk_geometry_in_cmos(
        &mut self,
        drive: u8,
        cylinders: u16,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .cmos
            .configure_disk_geometry(drive, cylinders, heads, spt);
    }

    /// Configure boot sequence in CMOS
    ///
    /// Boot device codes: 0=none, 1=floppy, 2=hard disk, 3=cdrom
    pub(crate) fn configure_boot_sequence(&mut self, first: u8, second: u8, third: u8) {
        self.device_manager
            .cmos
            .set_boot_sequence(first, second, third);
    }

    #[cfg(feature = "alloc")]
    /// Attach a CD-ROM ISO from in-memory data (for UEFI, WASM, or any environment).
    pub(crate) fn attach_cdrom_data(
        &mut self,
        channel: usize,
        drive: usize,
        data: alloc::vec::Vec<u8>,
    ) {
        self.device_manager
            .ide
            .drives
            .attach_cdrom_data(channel, drive, data);
    }

    #[cfg(feature = "alloc")]
    /// Attach a hard disk from in-memory data (for UEFI, WASM, or any environment).
    ///
    /// Wraps `HardDrive::attach_disk_data()` which stores the disk image
    /// in a `Vec<u8>` instead of using file I/O.
    pub(crate) fn attach_disk_data(
        &mut self,
        channel: usize,
        drive: usize,
        data: alloc::vec::Vec<u8>,
        cylinders: u32,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .ide
            .drives
            .attach_disk_data(channel, drive, data, cylinders, heads, spt);
    }

    /// Attach a CD-ROM ISO from a static byte slice (no-alloc).
    pub(crate) fn attach_cdrom_data_ref(
        &mut self,
        channel: usize,
        drive: usize,
        data: &'static [u8],
    ) {
        self.device_manager
            .ide
            .drives
            .attach_cdrom_data_ref(channel, drive, data);
    }

    /// Attach a hard disk from a static byte slice (no-alloc).
    pub(crate) fn attach_disk_data_ref(
        &mut self,
        channel: usize,
        drive: usize,
        data: &'static [u8],
        cylinders: u32,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .ide
            .drives
            .attach_disk_data_ref(channel, drive, data, cylinders, heads, spt);
    }

    /// Get the number of registered memory handlers (for diagnostics).
    pub fn memory_handler_count(&self) -> usize {
        self.memory.memory_handler_info()
    }

    /// Get current CS:RIP for diagnostics.
    pub fn get_cs_rip(&self) -> (u16, u64) {
        (self.cpu_ref(0).get_cs_selector(), self.cpu_ref(0).rip())
    }

    /// Get CPU mode string for diagnostics.
    pub fn get_cpu_mode_str(&self) -> &'static str {
        match self.cpu_ref(0).get_cpu_mode() {
            0 => "real",
            1 => "v8086",
            2 => "protected",
            3 => "long-compat",
            4 => "long-64",
            _ => "unknown",
        }
    }

    /// Get ATA channel read counters for diagnostics.
    pub fn ata_diag_reads(&self) -> (u64, u64) {
        (0, 0)
    }

    #[cfg(feature = "alloc")]
    /// Get ATA channel 1 (CD-ROM) controller state + interrupt routing diagnostics.
    pub fn ata_ch1_diag(&self) -> String {
        let ch1 = &self.device_manager.ide.drives.channels[1];
        let d = ch1.selected_drive();
        let (vec15, masked15, trig15, _dmode15) =
            self.device_manager.ioapic.redirect_entry_diag(15);
        // Check LAPIC IRR/ISR for the IDE vector
        let (irr_set, isr_set) = if vec15 > 0 {
            self.cpu_ref(0).lapic_vector_state(vec15)
        } else {
            (false, false)
        };
        format!("s={:?} cmd={:#04x} ip={} acmd={:#04x} nIEN={} IOAPIC15[v={:#04x} m={} t={}] LAPIC[irr={} isr={}]",
            d.controller.status, d.controller.current_command,
            d.controller.interrupt_pending,
            d.atapi.command,
            d.controller.control & 0x02,
            vec15, masked15 as u8, trig15,
            irr_set, isr_set)
    }

    /// Get total I/O port read/write counters for diagnostics.
    pub fn io_diag_counts(&self) -> (u64, u64) {
        (self.devices.diag_io_reads, self.devices.diag_io_writes)
    }

    /// Get CPU activity state and async_event for diagnostics.
    pub fn cpu_diag_state(&self) -> (u32, u32) {
        (self.cpu_ref(0).activity_state as u32, self.cpu_ref(0).async_event)
    }

    /// Get CR0 for diagnostics (bit 0 = PE).
    pub fn get_cr0(&self) -> u32 {
        self.cpu_ref(0).cr0.bits()
    }

    /// Get IF flag for diagnostics.
    pub fn get_if_flag(&self) -> bool {
        self.cpu_ref(0).get_b_if() != 0
    }

    /// Read a few bytes from the BIOS ROM array at the given ROM offset.
    pub fn peek_rom(&self, offset: usize, len: usize) -> &[u8] {
        self.memory.peek_rom(offset, len)
    }

    /// Get VGA Graphics Register 6 (memory mapping control).
    pub fn peek_vga_gr6(&self) -> u8 {
        self.device_manager.vga.graphics_regs[6]
    }

    /// Get CR3 (page directory base register) for page table walks.
    pub fn get_cr3(&self) -> u64 {
        self.cpu_ref(0).cr3
    }

    /// Get EIP for diagnostics.
    pub fn get_eip(&self) -> u32 {
        self.cpu_ref(0).eip()
    }

    /// Get segment register info: (selector, base, limit, valid_flags).
    pub fn get_seg_info(&self, seg_idx: usize) -> (u16, u64, u32, u32) {
        if seg_idx < 6 {
            let selector = self.cpu_ref(0).sregs[seg_idx].selector.value;
            let valid = self.cpu_ref(0).sregs[seg_idx].cache.valid;
            let base = self.cpu_ref(0).sregs[seg_idx].cache.u.segment_base();
            let limit = self.cpu_ref(0).sregs[seg_idx].cache.u.segment_limit_scaled();
            (selector, base, limit, valid)
        } else {
            (0, 0, 0, 0)
        }
    }

    /// Get EAX/EBX/ECX/EDX for diagnostics.
    pub fn get_gpr32(&self, reg: usize) -> u32 {
        match reg {
            0 => self.cpu_ref(0).eax(),
            1 => self.cpu_ref(0).ecx(),
            2 => self.cpu_ref(0).edx(),
            3 => self.cpu_ref(0).ebx(),
            4 => self.cpu_ref(0).esp(),
            5 => self.cpu_ref(0).ebp(),
            6 => self.cpu_ref(0).esi(),
            7 => self.cpu_ref(0).edi(),
            _ => 0,
        }
    }

    /// Get the activity state string.
    pub fn get_activity_str(&self) -> &'static str {
        match self.cpu_ref(0).activity_state {
            CpuActivityState::Active => "active",
            CpuActivityState::Hlt => "hlt",
            CpuActivityState::Shutdown => "shutdown",
            _ => "other",
        }
    }

    /// The DTLB slot a dword read at `laddr` would use.
    ///
    /// `ram_offset` is `None` when the page carries no direct mapping — it is
    /// MMIO, ROM, or outside guest RAM — which is exactly what the entry
    /// stores. Reporting the offset rather than a host address keeps this
    /// answerable without a live execution context, and is the unit the entry
    /// is actually keyed by (R4).
    pub fn get_dtlb_info(&self, laddr: u64) -> DtlbInfo {
        let idx = self.cpu_ref(0).dtlb.get_index_of(laddr, 3);
        let entry = &self.cpu_ref(0).dtlb.entries[idx];
        DtlbInfo {
            lpf: entry.lpf,
            ppf: entry.ppf,
            access_bits: entry.access_bits,
            ram_offset: entry.host_page.map(|page| page.ram_offset()),
        }
    }

    /// Get user_pl flag (true = CPL==3).
    pub fn get_user_pl(&self) -> bool {
        self.cpu_ref(0).user_pl
    }

    /// Read a physical dword through the block-aware RAM interface.
    /// Returns `None` when the complete range is unavailable.
    pub fn read_phys_dword(&mut self, paddr: u64) -> Option<u32> {
        let mut bytes = [0; 4];
        self.mem_read(paddr, &mut bytes).ok()?;
        Some(u32::from_le_bytes(bytes))
    }
}

impl<T: Instrumentation> Emulator<T> {
    /// Dump comprehensive diagnostic state (for Alpine debugging).
    #[cfg(all(feature = "std", debug_assertions))]
    pub fn dump_alpine_diag(&mut self) {
        tracing::trace!("\n=== DIAGNOSTIC DUMP ===");
        tracing::trace!(
            "RIP={:#018x} RSP={:#018x} RBP={:#018x}",
            self.cpu_ref(0).rip(),
            self.cpu_ref(0).rsp(),
            self.cpu_ref(0).rbp()
        );
        tracing::trace!(
            "RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
            self.cpu_ref(0).rax(),
            self.cpu_ref(0).rbx(),
            self.cpu_ref(0).rcx(),
            self.cpu_ref(0).rdx()
        );
        tracing::trace!(
            "RSI={:#018x} RDI={:#018x} R8={:#018x}  R9={:#018x}",
            self.cpu_ref(0).rsi(),
            self.cpu_ref(0).rdi(),
            self.cpu_ref(0).r8(),
            self.cpu_ref(0).r9()
        );
        tracing::trace!(
            "CS={:#06x} mode={} IF={}",
            self.cpu_ref(0).get_cs_selector(),
            self.get_cpu_mode_str(),
            if self.cpu_ref(0).get_b_if() != 0 { 1 } else { 0 }
        );
        tracing::trace!(
            "CR0={:#010x} CR3={:#018x}",
            self.cpu_ref(0).cr0.bits(),
            self.cpu_ref(0).cr3
        );
        tracing::trace!(
            "pending_event={:#010x} event_mask={:#010x} async_event={}",
            self.cpu_ref(0).pending_event,
            self.cpu_ref(0).event_mask,
            self.cpu_ref(0).async_event
        );
        #[cfg(debug_assertions)]
        {
            tracing::trace!(
                "diag: intr_delivered={} if_blocked={} pic_empty={}",
                self.cpu_ref(0).diag_hae_intr_delivered,
                self.cpu_ref(0).diag_hae_intr_if_blocked,
                self.cpu_ref(0).diag_hae_intr_pic_empty
            );
            // SYSCALL ring buffer
            tracing::trace!(
                "--- Last {} SYSCALLs (total={}, sysret={}, blocked={}) ---",
                self.cpu_ref(0).diag_syscall_ring_idx.min(32),
                self.cpu_ref(0).diag_syscall_count,
                self.cpu_ref(0).diag_sysret_count,
                self.cpu_ref(0)
                    .diag_syscall_count
                    .saturating_sub(self.cpu_ref(0).diag_sysret_count)
            );
            {
                let count = self.cpu_ref(0).diag_syscall_ring_idx.min(32);
                let start = self.cpu_ref(0).diag_syscall_ring_idx.saturating_sub(32);
                for i in start..self.cpu_ref(0).diag_syscall_ring_idx {
                    let (nr, arg0, ic) = self.cpu_ref(0).diag_syscall_ring[i % 32];
                    tracing::trace!("  syscall nr={} arg0={:#x} icount={}", nr, arg0, ic);
                }
            }
        }
        // PIC state
        tracing::trace!("--- PIC State ---");
        tracing::trace!(
            "  master: IMR={:#04x} IRR={:#04x} ISR={:#04x} has_int={}",
            self.device_manager.pic.master.imr,
            self.device_manager.pic.master.irr,
            self.device_manager.pic.master.isr,
            self.device_manager.pic.has_interrupt()
        );
        tracing::trace!(
            "  slave:  IMR={:#04x} IRR={:#04x} ISR={:#04x}",
            self.device_manager.pic.slave.imr,
            self.device_manager.pic.slave.irr,
            self.device_manager.pic.slave.isr
        );
        // PIT state
        let pit_c0 = &self.device_manager.pit.counters[0];
        tracing::trace!("--- PIT State ---");
        tracing::trace!(
            "  C0: mode={:?} count={} gate={} output={}",
            pit_c0.mode,
            pit_c0.count,
            pit_c0.gate,
            pit_c0.output
        );
        tracing::trace!("--- Exact Timer Diag ---");
        tracing::trace!(
            "  pit_fires={} irq0_latched={} iac_count={}",
            self.device_manager.pit.diag_fires,
            self.device_manager.pit.diag_irq0_latched,
            self.device_manager.diag_iac_count
        );
        tracing::trace!(
            "  lapic_timer_fires={} set_initial_count={} timer_masked={}",
            self.cpu_ref(0).lapic.diag_timer_fires,
            self.cpu_ref(0).lapic.diag_set_initial_count,
            self.cpu_ref(0).lapic.diag_timer_masked
        );
        // Show pc_system timer state for LAPIC timer
        if let Some(handle) = self.cpu_ref(0).lapic.timer_handle {
            let t = &self.pc_system.timers[handle];
            tracing::trace!(
                "  pc_system_timer[{}]: flags={:?} time_to_fire={} period={} ticks_total={}",
                handle,
                t.flags,
                t.time_to_fire,
                t.period,
                self.pc_system.time_ticks()
            );
        }
        self.cpu_ref(0).lapic.dump_state();
        // ATA channel diagnostics
        tracing::trace!("--- ATA Diag ---");
        tracing::trace!("  cmd_history (last 10):");
        let hist: Vec<(u8, u8, u32)> = self.device_manager.ide.drives.cmd_history.iter().collect();
        let start = if hist.len() > 10 { hist.len() - 10 } else { 0 };
        for (ch, cmd, lba) in &hist[start..] {
            tracing::trace!("    ch={} cmd={:#04x} lba={}", ch, cmd, lba);
        }
        // Dump key code addresses through requested-size block-aware copies.
        {
            let addrs: &[(u64, &str)] = &[
                (0x01e1d340, "delay_loop_entry"),
                (0x01e38ef0, "jmp_target_after_delay"),
                (0x01207430, "outer_loop_context"),
                (0x01207460, "stack_ret_addr_1"),
                (0x012074e0, "stack_ret_addr_2"),
            ];
            for (paddr, label) in addrs {
                let code = self.peek_ram_at(*paddr as usize, 48);
                if code.len() == 48 {
                    tracing::trace!("--- {} (phys={:#010x}) ---", label, paddr);
                    for row in 0..3 {
                        let off = row * 16;
                        tracing::trace!("  +{:02x}: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}  {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                            off,
                            code[off], code[off+1], code[off+2], code[off+3],
                            code[off+4], code[off+5], code[off+6], code[off+7],
                            code[off+8], code[off+9], code[off+10], code[off+11],
                            code[off+12], code[off+13], code[off+14], code[off+15]);
                    }
                }
            }
        }
        // Dump stack (16 qwords) through a manual page walk that reads only
        // the individual page-table entries and stack words it needs.
        let rsp = self.cpu_ref(0).rsp();
        if rsp > 0xffffffff80000000 {
            let cr3 = self.cpu_ref(0).cr3 & !0xFFF;
            let mut read_stack_qword = |addr: u64| -> u64 {
                let pml4_idx = (addr >> 39) & 0x1FF;
                let pdpt_idx = (addr >> 30) & 0x1FF;
                let pd_idx = (addr >> 21) & 0x1FF;
                let pt_idx = (addr >> 12) & 0x1FF;
                let page_off = addr & 0xFFF;
                let pml4e = self.read_physical_u64_or_zero(cr3 + pml4_idx * 8);
                if pml4e & 1 == 0 {
                    return 0;
                }
                let pdpte =
                    self.read_physical_u64_or_zero((pml4e & 0xFFFFF_FFFFF000) + pdpt_idx * 8);
                if pdpte & 1 == 0 {
                    return 0;
                }
                if pdpte & 0x80 != 0 {
                    return self.read_physical_u64_or_zero(
                        (pdpte & 0xFFFFF_C0000000) | (addr & 0x3FFFFFFF),
                    );
                }
                let pde = self.read_physical_u64_or_zero((pdpte & 0xFFFFF_FFFFF000) + pd_idx * 8);
                if pde & 1 == 0 {
                    return 0;
                }
                if pde & 0x80 != 0 {
                    return self
                        .read_physical_u64_or_zero((pde & 0xFFFFF_FFE00000) | (addr & 0x1FFFFF));
                }
                let pte = self.read_physical_u64_or_zero((pde & 0xFFFFF_FFFFF000) + pt_idx * 8);
                if pte & 1 == 0 {
                    return 0;
                }
                self.read_physical_u64_or_zero((pte & 0xFFFFF_FFFFF000) | page_off)
            };
            tracing::trace!("--- Stack at RSP={:#018x} ---", rsp);
            for i in 0..16 {
                let addr = rsp.wrapping_add(i * 8);
                let val = read_stack_qword(addr);
                let marker = if val > 0xffffffff81000000 && val < 0xffffffff82000000 {
                    " <-- kernel text?"
                } else {
                    ""
                };
                tracing::trace!("  [{:+4}] {:#018x}{}", i * 8, val, marker);
            }
        }
        // Dump 64 bytes of code at current RIP via the same requested-size
        // physical reads used above.
        let rip = self.cpu_ref(0).rip();
        if rip > 0xffffffff80000000 {
            let cr3 = self.cpu_ref(0).cr3 & !0xFFF;
            let pml4_idx = (rip >> 39) & 0x1FF;
            let pdpt_idx = (rip >> 30) & 0x1FF;
            let pd_idx = (rip >> 21) & 0x1FF;
            let pt_idx = (rip >> 12) & 0x1FF;
            let pml4e = self.read_physical_u64_or_zero(cr3 + pml4_idx * 8);
            if pml4e & 1 != 0 {
                let pdpte =
                    self.read_physical_u64_or_zero((pml4e & 0x000FFFFF_FFFFF000) + pdpt_idx * 8);
                if pdpte & 1 != 0 {
                    let paddr = if pdpte & 0x80 != 0 {
                        (pdpte & 0x000FFFFF_C0000000) | (rip & 0x3FFFFFFF)
                    } else {
                        let pde = self
                            .read_physical_u64_or_zero((pdpte & 0x000FFFFF_FFFFF000) + pd_idx * 8);
                        if pde & 1 != 0 {
                            if pde & 0x80 != 0 {
                                (pde & 0x000FFFFF_FFE00000) | (rip & 0x1FFFFF)
                            } else {
                                let pte = self.read_physical_u64_or_zero(
                                    (pde & 0x000FFFFF_FFFFF000) + pt_idx * 8,
                                );
                                if pte & 1 != 0 {
                                    (pte & 0x000FFFFF_FFFFF000) | (rip & 0xFFF)
                                } else {
                                    0
                                }
                            }
                        } else {
                            0
                        }
                    };
                    let code = self.peek_ram_at(paddr as usize, 64);
                    if paddr != 0 && code.len() == 64 {
                        tracing::trace!("--- Code at RIP={:#018x} (phys={:#010x}) ---", rip, paddr);
                        for row in 0..4 {
                            let off = row * 16;
                            tracing::trace!("  {:016x}: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}  {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                                rip + off as u64,
                                code[off], code[off+1], code[off+2], code[off+3],
                                code[off+4], code[off+5], code[off+6], code[off+7],
                                code[off+8], code[off+9], code[off+10], code[off+11],
                                code[off+12], code[off+13], code[off+14], code[off+15]);
                        }
                    }
                }
            }
        }
        tracing::trace!("=== END DIAGNOSTIC ===");
    }
}

#[cfg(feature = "std")]
#[inline]
fn status_ips_from_retired_instructions(
    last_instructions: u64,
    current_instructions: u64,
    elapsed: std::time::Duration,
) -> u32 {
    if elapsed.is_zero() {
        return 0;
    }

    let ips = current_instructions.saturating_sub(last_instructions) as f64 / elapsed.as_secs_f64();
    ips.clamp(0.0, u32::MAX as f64) as u32
}

// `Send` is derived, not promised (doctrine R6). A machine owns every part it
// runs and reaches each one by field borrow, so there is no pointer left for a
// hand-written impl to vouch for. A field that reintroduced one would break
// this line rather than silently un-thread-safe the fleet.
//
// This holds in EVERY build. The no-alloc machine used to be exempt because it
// kept its CPUs as `*mut BxCpuC`; it now borrows them exclusively instead, and
// an exclusive borrow of a `Send` type is itself `Send`. There is no longer a
// configuration of this emulator that cannot cross a thread boundary.
const _: () = {
    const fn assert_send<M: Send>() {}
    assert_send::<Emulator<()>>();
};

/// Non-vacuity for the assertion above: the machine stays `Send` for any
/// `Send` tracer, not just the `()` default.
#[allow(dead_code)]
fn assert_machine_send_for_every_send_tracer<T: Instrumentation + Send>() {
    const fn assert_send<M: Send>() {}
    assert_send::<Emulator<T>>();
}

#[cfg(all(test, feature = "alloc"))]
mod tests;
