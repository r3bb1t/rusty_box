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
#[cfg(feature = "alloc")]
use crate::{
    cpu::builder::BxCpuBuilder, iodev::acpi_tables::AcpiTableGenerator, memory::MemoryError,
};
use crate::{
    cpu::{
        cpu::CpuActivityState,
        instrumentation::{ExitSet, Instrumentation},
        BxCpuC, CpuError, CpuidFreq, ResetReason,
    },
    iodev::{
        devices::{DeviceManager, SystemControlPort},
        BxDevicesC,
    },
    memory::{BxMemC, BxMemoryStubC, CpuTlbPin},
    params::BxParams,
    pc_system::BxPcSystemC,
    Error, Result,
};
#[cfg(feature = "std")]
use crate::pc_system::TimerOwner;

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, format, string::String, sync::Arc, vec::Vec};
#[cfg(not(feature = "alloc"))]
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicBool;

mod interactive;
mod run;
mod scheduler;
mod timers;

#[cfg(not(feature = "alloc"))]
const NO_ALLOC_MAX_AP_CPUS: usize = (crate::params::BX_MAX_SMP_THREADS_SUPPORTED as usize) - 1;
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

/// Emulator configuration
#[derive(Debug, Clone)]
pub struct EmulatorConfig {
    /// Guest memory size in bytes
    pub guest_memory_size: usize,
    /// Host memory size in bytes (can be less than guest for swapping)
    pub host_memory_size: usize,
    /// Memory block size for allocation
    pub memory_block_size: usize,
    /// Emulated instructions per second, used to calibrate emulated time
    /// against wall-clock time. Bochs config.cc raised its default from 4M to
    /// 50M — 4M badly under-reports modern hosts, which makes every guest
    /// timeout fire early.
    pub ips: u32,
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
            guest_memory_size: 32 * 1024 * 1024,
            host_memory_size: 32 * 1024 * 1024,
            memory_block_size: 128 * 1024,
            ips: 50_000_000,
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
        let total_emulated_usec =
            emulated_time_usec.saturating_sub(self.start_emulated_time_usec);
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
/// ```ignore
/// use rusty_box::emulator::{Emulator, EmulatorConfig};
/// use rusty_box::cpu::core_i7_skylake::Corei7SkylakeX;
///
/// let config = EmulatorConfig::default();
/// let mut emu = Emulator::new(config)?;
/// emu.initialize()?;
/// emu.load_bios(&bios_data, 0xfffe0000)?;
/// emu.reset(ResetReason::Hardware)?;
/// // Read architectural state through `cpu()` and mutate it through targeted
/// // emulator operations such as `reg_write()` and `reset()`.
/// assert_eq!(emu.cpu().rip(), 0);
/// ```
///
/// The memory backing is intentionally not publicly replaceable:
///
/// ```compile_fail
/// use rusty_box::cpu::core_i7_skylake::Corei7SkylakeX;
/// use rusty_box::emulator::{Emulator, EmulatorConfig};
///
/// let mut emu = Emulator::new(EmulatorConfig::default()).unwrap();
/// let _ = &mut emu.memory;
/// ```
/// Caller-provided BSP storage for no-alloc builds.
///
/// The alloc build keeps its BSP in a `Box`, which owns the CPU and needs no
/// lifetime. No-alloc callers place theirs in a static, on the stack, or in
/// firmware-provided memory, and previously handed it over as `&'a mut` — which
/// forced a lifetime parameter onto `Emulator` that the alloc build had nothing
/// to fill. Holding it as a pointer removes that asymmetry, and matches
/// `ap_cpu_ptrs`, which has always stored the other CPUs this way.
///
/// The dereference lives here and nowhere else, so every use site reads
/// `self.cpu.field` exactly as it does under alloc.
#[cfg(not(feature = "alloc"))]
pub(crate) struct BspCpu<T: Instrumentation>(*mut BxCpuC<T>);

#[cfg(not(feature = "alloc"))]
impl<T: Instrumentation> BspCpu<T> {
    #[inline(always)]
    pub(crate) fn new(cpu: &mut BxCpuC<T>) -> Self {
        Self(cpu)
    }
}

#[cfg(not(feature = "alloc"))]
impl<T: Instrumentation> core::ops::Deref for BspCpu<T> {
    type Target = BxCpuC<T>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // SAFETY: the caller of `init_at` guarantees this CPU outlives the
        // emulator — the same contract `ap_cpu_ptrs` already rests on.
        unsafe { &*self.0 }
    }
}

#[cfg(not(feature = "alloc"))]
impl<T: Instrumentation> core::ops::DerefMut for BspCpu<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: see `Deref`; `&mut self` excludes every other borrow.
        unsafe { &mut *self.0 }
    }
}

pub struct Emulator<T: Instrumentation = ()> {
    /// BSP CPU storage. This stays at a stable address for its own cached
    /// host mappings; eviction-visible state instead lives in `cpu_tlb_pins`.
    #[cfg(feature = "alloc")]
    cpu: alloc::boxed::Box<BxCpuC<T>>,
    /// Application processors (CPU IDs/APIC IDs 1..N-1).
    #[cfg(feature = "alloc")]
    pub(crate) ap_cpus: Vec<alloc::boxed::Box<BxCpuC<T>>>,
    /// Stable descriptors for every CPU's direct host-memory references.
    #[cfg(feature = "alloc")]
    cpu_tlb_pins: Vec<CpuTlbPin>,
    /// BSP CPU storage supplied by no-alloc callers. Its external pin sidecar
    /// is stored separately in the fixed descriptor array below.
    #[cfg(not(feature = "alloc"))]
    cpu: BspCpu<T>,
    /// Application processor pointers supplied by no-alloc callers.
    ///
    /// no_std/no-alloc targets can place `BxCpuC` objects in a static, stack,
    /// firmware, or bootloader-provided array and pass those references through
    /// `init_at_with_ap_cpus()`. The emulator stores raw pointers so it does not
    /// need `alloc` or `std` to support SMP scheduling.
    #[cfg(not(feature = "alloc"))]
    ap_cpu_ptrs: [*mut BxCpuC<T>; NO_ALLOC_MAX_AP_CPUS],
    #[cfg(not(feature = "alloc"))]
    ap_cpu_count: usize,
    #[cfg(not(feature = "alloc"))]
    cpu_tlb_pins: [MaybeUninit<CpuTlbPin>; NO_ALLOC_MAX_AP_CPUS + 1],
    #[cfg(not(feature = "alloc"))]
    cpu_tlb_pin_count: usize,
    /// Memory subsystem
    pub(crate) memory: BxMemC,
    /// Device controller (I/O port handlers)
    pub devices: BxDevicesC,
    /// Device manager (actual hardware devices)
    pub device_manager: DeviceManager,
    /// PC system (timers, A20, etc.)
    pub pc_system: BxPcSystemC,
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
    /// Shared stop flag: when set to true by the GUI thread, run_interactive exits the loop
    #[cfg(feature = "alloc")]
    pub stop_flag: Arc<AtomicBool>,
    #[cfg(not(feature = "alloc"))]
    pub stop_flag: AtomicBool,
}

impl<'a, T: Instrumentation> Emulator<T> {
    #[cfg(feature = "alloc")]
    pub(crate) fn cpu_count(&self) -> usize {
        1 + self.ap_cpus.len()
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn cpu_count(&self) -> usize {
        1 + self.ap_cpu_count
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn cpu_ref(&self, index: usize) -> &BxCpuC<T> {
        if index == 0 {
            &self.cpu
        } else {
            &self.ap_cpus[index - 1]
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn cpu_ref(&self, index: usize) -> &BxCpuC<T> {
        if index == 0 {
            &*self.cpu
        } else {
            assert!(index <= self.ap_cpu_count);
            // SAFETY: init_at_with_ap_cpus copies caller-provided AP pointers
            // whose allocations must outlive the emulator.
            unsafe { &*self.ap_cpu_ptrs[index - 1] }
        }
    }


    #[cfg(feature = "alloc")]
    pub(crate) fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<T> {
        if index == 0 {
            &mut self.cpu
        } else {
            &mut self.ap_cpus[index - 1]
        }
    }

    /// Borrow one CPU together with the machine it executes against.
    ///
    /// This is the whole point of `ExecCtx`: the CPU, memory, devices, PC
    /// system and pin set are separate fields, so a single destructuring hands
    /// out `&mut` to each simultaneously. The scheduler's raw `mem_ptr` /
    /// `io_ptr` / `ps_ptr` wiring exists only because it re-borrows `self`
    /// inside its loop; nothing about the data itself requires a pointer.
    #[cfg(feature = "alloc")]
    pub(crate) fn exec_ctx(&mut self, index: usize) -> crate::cpu::exec_ctx::ExecCtx<'_, T> {
        let Self {
            cpu,
            ap_cpus,
            memory,
            devices,
            pc_system,
            cpu_tlb_pins,
            ..
        } = self;
        let this_cpu: &mut BxCpuC<T> = if index == 0 {
            cpu
        } else {
            &mut ap_cpus[index - 1]
        };
        crate::cpu::exec_ctx::ExecCtx::new(
            this_cpu,
            memory,
            devices,
            pc_system,
            &cpu_tlb_pins[..],
            index,
        )
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<T> {
        if index == 0 {
            &mut self.cpu
        } else {
            assert!(index <= self.ap_cpu_count);
            // SAFETY: &mut self guarantees no other emulator method can
            // concurrently borrow the AP CPU through this pointer.
            unsafe { &mut *self.ap_cpu_ptrs[index - 1] }
        }
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
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()))?;

        let sci_level = self
            .device_manager
            .acpi
            .post_restore_snapshot_v3(self.pc_system.time_ticks());
        self.device_manager.serial.after_restore_snapshot_v3()?;
        self.device_manager.vga.rebuild_snapshot_v3_derived_state()?;
        self.validate_restored_irq_levels(&keyboard, &cmos, sci_level)?;
        self.sync_restored_event_levels();
        self.rebuild_cpu_masks_from_scan();
        self.batch_advanced_pc_system = false;
        self.clear_scheduler_raw_wiring();
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
            if pic.irq_line_level(irq) != self.device_manager.ide.drives.get_irq_level(channel)
            {
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
impl<'a> Emulator<()> {
    /// Create a new emulator with no instrumentation (`T = ()`).
    ///
    /// Returns `Box<Self>` because Emulator is ~1.4 MB.
    pub fn new(config: EmulatorConfig) -> Result<Box<Self>> {
        Self::new_inner(config, || Ok(BxCpuBuilder::new().build()?))
    }
}

#[cfg(feature = "alloc")]
impl<'a, T: Instrumentation> Emulator<T> {
    /// Create a new emulator with a monomorphized tracer.
    ///
    /// The tracer type `T` is baked in at construction and cannot be changed.
    /// All tracer dispatch is inlined — zero overhead.
    pub fn new_with_instrumentation(config: EmulatorConfig, tracer: T) -> Result<Box<Self>> {
        if config.cpu_params.cpu_count() > 1 {
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "instrumented SMP construction requires a per-CPU tracer factory",
            }
            .into());
        }

        let topology = config.cpu_params.cpu_topology();
        let mut cpu = BxCpuBuilder::new().build_with_tracer(tracer)?;
        cpu.configure_smp(0, topology);
        cpu.set_smp_quantum(config.smp_quantum);
        cpu.set_cpuid_freq(config.cpuid_freq, config.ips);
        Self::new_from_parts(config, cpu, Vec::new())
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
        cpu.set_cpuid_freq(config.cpuid_freq, config.ips);

        let mut ap_cpus = Vec::with_capacity(cpu_count.saturating_sub(1) as usize);
        for cpu_id in 1..cpu_count {
            let mut ap_cpu = build_cpu()?;
            ap_cpu.configure_smp(cpu_id, topology);
            ap_cpu.set_smp_quantum(config.smp_quantum);
            ap_cpu.set_cpuid_freq(config.cpuid_freq, config.ips);
            ap_cpus.push(ap_cpu);
        }
        Self::new_from_parts(config, cpu, ap_cpus)
    }

    fn new_from_parts(
        config: EmulatorConfig,
        cpu: alloc::boxed::Box<BxCpuC<T>>,
        ap_cpus: Vec<alloc::boxed::Box<BxCpuC<T>>>,
    ) -> Result<Box<Self>> {
        let mut cpu_tlb_pins = Vec::new();
        cpu_tlb_pins
            .try_reserve_exact(1 + ap_cpus.len())
            .map_err(|_| MemoryError::UnableToAllocateGuestMemory(core::mem::size_of::<CpuTlbPin>()))?;
        // This is the descriptor's final backing allocation. No CPU scope
        // receives a sidecar pointer until every element is populated, and
        // this Vec is never grown afterwards.
        cpu_tlb_pins.push(CpuTlbPin::new(&cpu));
        for ap_cpu in &ap_cpus {
            cpu_tlb_pins.push(CpuTlbPin::new(ap_cpu));
        }
        let pc_system = BxPcSystemC::new();
        let mem_stub = BxMemoryStubC::create_and_init(
            config.guest_memory_size,
            config.host_memory_size,
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
            core::ptr::addr_of_mut!((*ptr).cpu).write(cpu);
            core::ptr::addr_of_mut!((*ptr).ap_cpus).write(ap_cpus);
            core::ptr::addr_of_mut!((*ptr).cpu_tlb_pins).write(cpu_tlb_pins);
            core::ptr::addr_of_mut!((*ptr).memory).write(memory);
            core::ptr::addr_of_mut!((*ptr).devices).write(devices);
            core::ptr::addr_of_mut!((*ptr).device_manager).write(device_manager);
            core::ptr::addr_of_mut!((*ptr).pc_system).write(pc_system);
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
            Ok(alloc::boxed::Box::from_raw(ptr))
        }
    }
}

impl<'a, T: Instrumentation> Emulator<T> {
    #[cfg(not(feature = "alloc"))]
    /// Initialize an Emulator at a caller-provided memory location.
    ///
    /// In no-alloc environments the caller is responsible for allocating and
    /// initializing the `BxMemoryStubC` (e.g. from a firmware-provided buffer).
    ///
    /// # Safety
    /// - `ptr` must point to a valid, zeroed, properly aligned allocation of `size_of::<Self>()` bytes
    /// - `cpu` must point to a valid, initialized BxCpuC
    /// - `mem_stub` must be a fully initialized memory stub
    /// - All allocations must outlive the returned reference
    pub unsafe fn init_at(
        ptr: *mut Self,
        cpu: &'a mut BxCpuC<T>,
        mem_stub: BxMemoryStubC,
        config: EmulatorConfig,
    ) -> Result<&'a mut Self> {
        Self::init_at_with_ap_cpus(ptr, cpu, &mut [], mem_stub, config)
    }

    #[cfg(not(feature = "alloc"))]
    /// Initialize an Emulator at a caller-provided memory location with
    /// caller-provided application processors.
    ///
    /// This keeps SMP available to no_std/no-alloc targets: callers can define
    /// a fixed array of `BxCpuC` storage, initialize each CPU with
    /// `BxCpuBuilder::init_cpu_at()`, then pass the AP references here. The
    /// emulator stores raw pointers to the APs and never allocates.
    ///
    /// # Safety
    /// - `ptr` must point to a valid, zeroed, properly aligned allocation of `size_of::<Self>()` bytes
    /// - `cpu` and every entry in `ap_cpus` must point to valid, initialized BxCpuC instances
    /// - `mem_stub` must be a fully initialized memory stub
    /// - All allocations must outlive the returned emulator reference
    pub unsafe fn init_at_with_ap_cpus(
        ptr: *mut Self,
        cpu: &'a mut BxCpuC<T>,
        ap_cpus: &mut [&'a mut BxCpuC<T>],
        mem_stub: BxMemoryStubC,
        config: EmulatorConfig,
    ) -> Result<&'a mut Self> {
        let topology = config.cpu_params.cpu_topology();
        let configured_cpu_count = config.cpu_params.cpu_count() as usize;
        let required_ap_count = configured_cpu_count.saturating_sub(1);
        if ap_cpus.len() < required_ap_count {
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "no-alloc SMP requires caller-provided AP CPU storage",
            }
            .into());
        }

        let memory = BxMemC::new_from_stub(mem_stub, config.pci_enabled);
        let devices = BxDevicesC::new();
        let device_manager = DeviceManager::new();
        let pc_system = BxPcSystemC::new();
        let mut ap_cpu_ptrs = [core::ptr::null_mut(); NO_ALLOC_MAX_AP_CPUS];
        cpu.configure_smp(0, topology);
        cpu.set_smp_quantum(config.smp_quantum);
        cpu.set_cpuid_freq(config.cpuid_freq, config.ips);
        for (index, ap_cpu_slot) in ap_cpus.iter_mut().take(required_ap_count).enumerate() {
            let ap_cpu: &mut BxCpuC<T> = &mut **ap_cpu_slot;
            ap_cpu.configure_smp((index + 1) as u32, topology);
            ap_cpu.set_smp_quantum(config.smp_quantum);
            ap_cpu.set_cpuid_freq(config.cpuid_freq, config.ips);
            ap_cpu_ptrs[index] = ap_cpu as *mut BxCpuC<T>;
        }
        // The descriptor sidecars are 40 KiB each. Initialize only the used
        // prefix directly in the caller-provided Emulator storage so no-alloc
        // construction neither allocates nor builds/moves a 254-entry stack
        // temporary. Sidecar addresses become stable before any CPU scope
        // wires one into `active_tlb_pin_sidecar`.
        let bsp_ptr = cpu as *mut BxCpuC<T>;
        core::ptr::addr_of_mut!((*ptr).cpu).write(BspCpu::new(cpu));
        core::ptr::addr_of_mut!((*ptr).ap_cpu_ptrs).write(ap_cpu_ptrs);
        core::ptr::addr_of_mut!((*ptr).ap_cpu_count).write(required_ap_count);
        let pin_slots = core::ptr::addr_of_mut!((*ptr).cpu_tlb_pins)
            .cast::<MaybeUninit<CpuTlbPin>>();
        pin_slots.write(MaybeUninit::new(CpuTlbPin::new(&*bsp_ptr)));
        for index in 0..required_ap_count {
            pin_slots
                .add(index + 1)
                .write(MaybeUninit::new(CpuTlbPin::new(&*ap_cpu_ptrs[index])));
        }
        core::ptr::addr_of_mut!((*ptr).cpu_tlb_pin_count).write(required_ap_count + 1);
        core::ptr::addr_of_mut!((*ptr).memory).write(memory);
        core::ptr::addr_of_mut!((*ptr).devices).write(devices);
        core::ptr::addr_of_mut!((*ptr).device_manager).write(device_manager);
        core::ptr::addr_of_mut!((*ptr).pc_system).write(pc_system);
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
        Ok(&mut *ptr)
    }

    fn configure_pci_devices(&mut self) {
        self.devices.set_pci_enabled(self.config.pci_enabled);
        let ramsize_mb = (self.config.guest_memory_size / (1024 * 1024)) as u32;
        self.device_manager.pci_bridge.init_dram(ramsize_mb);
        if self.config.pci_enabled && self.config.pci_vga {
            self.device_manager.vga.enable_pci();
            tracing::info!("VGA registered as PCI device (1234:1111, class 0300)");
        }
        tracing::trace!("PCI bridge DRAM initialized for {}MB", ramsize_mb);
    }

    #[cfg(feature = "alloc")]
    pub fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            tracing::trace!("Emulator already initialized");
            return Ok(());
        }

        tracing::debug!("Initializing emulator");

        // Step 1: Initialize PC system with IPS (line 1201)
        self.pc_system.initialize(self.config.ips);
        self.devices.set_timer_ips(u64::from(self.config.ips));
        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        tracing::trace!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
        self.invalidate_all_cpu_host_mappings();
        self.memory.init_memory(
            self.config.guest_memory_size,
            self.config.host_memory_size,
            self.config.memory_block_size,
        )?;

        // Sync A20 mask from PC system (after memory init, matching original)
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        tracing::trace!("Memory initialized and A20 mask synced");

        // Step 3-5: BIOS/ROM/RAM loading should happen HERE (after memory init, before CPU init)
        // But since this method doesn't have BIOS data, it's loaded separately after this call.
        // For correct initialization, use init_memory() + load_bios() + init_cpu_and_devices()

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
            let ram_size = self.config.guest_memory_size as u64;
            let cpu_count = self.config.cpu_params.cpu_count();
            self.device_manager.ioapic.set_id(cpu_count);
            self.device_manager.fw_cfg.init(ram_size, cpu_count);
            let acpi = AcpiTableGenerator::generate(ram_size, cpu_count);
            self.device_manager.fw_cfg.add_acpi_tables(
                acpi.tables_blob(),
                acpi.rsdp_blob(),
                acpi.loader_blob(),
            );
        }
        tracing::trace!("Devices initialized");

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

    /// Initialize memory and PC system (Step 1-2 of initialization)
    ///
    /// This is the first part of the initialization sequence from Bochs main.cc:
    /// 1. PC system initialization (timers, IPS) - line 1201
    /// 2. Memory initialization - line 1312
    ///
    /// After this, call `load_bios()` and `load_optional_rom()`, then `init_cpu_and_devices()`.
    /// This matches the original Bochs sequence: Memory init → Load BIOS → CPU init → Device init.
    #[cfg(feature = "alloc")]
    pub fn init_memory_and_pc_system(&mut self) -> Result<()> {
        if self.initialized {
            tracing::trace!("Emulator already initialized");
            return Ok(());
        }

        tracing::debug!("Initializing hardware...");

        // Step 1: Initialize PC system with IPS (line 1201)
        self.pc_system.initialize(self.config.ips);
        self.devices.set_timer_ips(u64::from(self.config.ips));
        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        tracing::trace!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
        self.invalidate_all_cpu_host_mappings();
        self.memory.init_memory(
            self.config.guest_memory_size,
            self.config.host_memory_size,
            self.config.memory_block_size,
        )?;

        // Sync A20 mask from PC system (after memory init, matching original)
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        tracing::trace!("Memory initialized and A20 mask synced");

        Ok(())
    }

    /// Initialize PC system timers and sync A20 mask.
    /// Use this instead of `init_memory_and_pc_system` when memory was
    /// initialized externally (e.g. via `init_at`).
    pub fn init_pc_system(&mut self) {
        self.pc_system.initialize(self.config.ips);
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
    pub fn init_cpu_and_devices(&mut self) -> Result<()> {
        // The no-alloc construction path (`init_at`) has no separate
        // `init_memory_and_pc_system` step, so make this initializer
        // self-sufficient: without it a no-alloc machine ran every device
        // timer conversion against the default `ips = 1`. Re-running it in
        // the alloc flow is harmless — no timers are registered until
        // `register_timer_owners` below and no virtual time has advanced.
        self.pc_system.initialize(self.config.ips);
        self.devices.set_timer_ips(u64::from(self.config.ips));
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
            let ram_size = self.config.guest_memory_size as u64;
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
    /// Set the GUI instance
    ///
    /// Based on load_and_init_display_lib() in main.cc
    pub fn set_gui<G: BxGui + 'static>(&mut self, gui: G) {
        self.gui = Some(Box::new(gui));
        tracing::debug!("GUI set");
    }

    #[cfg(feature = "alloc")]
    /// Initialize the GUI
    ///
    /// Based on bx_init_hardware() GUI initialization in main.cc
    /// This calls specific_init() to set up the GUI, but signal handlers are
    /// initialized separately via init_gui_signal_handlers() after reset.
    pub fn init_gui(&mut self, argc: i32, argv: &[&str]) -> Result<()> {
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
    /// without allowing safe replacement of the pinned CPU storage.
    #[inline]
    pub(crate) fn cpu_mut(&mut self) -> &mut BxCpuC<T> {
        self.cpu_mut_at(0)
    }

    /// Mutably access the pinned BSP CPU without moving it.
    ///
    /// Prefer the targeted safe `Emulator` operations whenever one exists.
    /// This escape hatch is for external integrations that need arbitrary CPU
    /// state mutation.
    ///
    /// # Safety
    ///
    /// Pin descriptors do not point at this CPU: their external sidecars are
    /// refreshed before each memory scope. The caller must not move, replace,
    /// swap, or retain stale references/raw pointers obtained from the CPU
    /// beyond their valid borrow and emulator lifetimes.
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
    pub fn load_bios(&mut self, bios_data: &[u8], address: u64) -> Result<()> {
        self.memory.load_ROM(bios_data, address, 0)?;
        tracing::debug!("Loaded BIOS ({} bytes) at {:#x}", bios_data.len(), address);
        Ok(())
    }

    /// Load an optional ROM image (VGA BIOS, expansion ROMs, etc.)
    ///
    /// # Arguments
    /// * `rom_data` - Raw ROM data
    /// * `address` - Load address (must be in 0xC0000-0xFFFFF range)
    pub fn load_optional_rom(&mut self, rom_data: &[u8], address: u64) -> Result<()> {
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
    pub fn load_ram(&mut self, ram_data: &[u8], address: u64) -> Result<()> {
        let pins_ptr = self.tlb_pins().as_ptr();
        let pins_len = self.tlb_pins().len();
        // Stable CPU pin storage outlives this exclusive memory borrow.
        let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
        self.memory.load_RAM(pins, ram_data, address)?;
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
    pub fn init_gui_signal_handlers(&mut self) {
        if let Some(ref mut gui) = self.gui {
            gui.init_signal_handlers();
            tracing::trace!("GUI signal handlers initialized");
        }
    }

    /// Start timers and prepare for execution
    /// Note: Timers are now started in reset(), so this is mostly for compatibility
    pub fn start(&mut self) {
        self.pc_system.start_timers();
        tracing::trace!("Timers started");
    }

    /// Check if the emulator is ready to run
    ///
    /// Call this before accessing `cpu.cpu_loop()`.
    pub fn ready_to_run(&self) -> Result<()> {
        if !self.initialized {
            return Err(Error::Cpu(CpuError::CpuNotInitialized));
        }
        Ok(())
    }

    /// Prepare for execution (start timers and log)
    ///
    /// Call this before entering the CPU loop.
    pub fn prepare_run(&mut self) {
        tracing::trace!("Starting CPU execution at RIP={:#x}", self.cpu.rip());

        // Initialize PIT icount sync so PIT counter reads advance with CPU time.
        // This is critical for kernel PIT-polling calibration loops (e.g., Alpine Linux).
        let ips = self.config.ips as u64;
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
            let ips = self.config.ips as u64;
            self.device_manager.vga.set_icount_sync(ips);
        }

        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        self.start();
    }

    /// Get current instruction pointer
    pub fn rip(&self) -> u64 {
        self.cpu.rip()
    }

    #[cfg(feature = "alloc")]
    /// Return the current VGA text-mode screen as a string.
    ///
    /// This is useful for headless debugging (no terminal repaint).
    pub fn vga_text_dump(&self) -> String {
        self.device_manager.vga.get_text_screen()
    }

    #[cfg(feature = "alloc")]
    pub fn vga_probe_dump(&self) -> String {
        self.device_manager.vga.probe_summary()
    }

    #[cfg(feature = "alloc")]
    /// Scan all VGA text memory for any non-space printable characters.
    /// Useful when the screen has been cleared and we need to find if a new
    /// prompt was written somewhere in text_memory that the CRTC start address
    /// may not be pointing to yet.
    pub fn vga_scan_text_memory(&self) -> String {
        self.device_manager.vga.scan_all_text_memory()
    }

    #[cfg(feature = "alloc")]
    /// Return all rows from VGA text memory (for full-dump diagnostics).
    pub fn vga_all_text_rows(&self) -> alloc::vec::Vec<alloc::string::String> {
        self.device_manager.vga.get_all_text_rows()
    }

    #[cfg(feature = "alloc")]
    /// Read up to `len` physical-RAM bytes for diagnostics.
    ///
    /// The result is intentionally a requested-size copy: guest RAM can be
    /// block-backed and swapped, so it is never exposed as a borrowed slice.
    pub fn peek_ram_at(&mut self, addr: usize, len: usize) -> alloc::vec::Vec<u8> {
        let mut bytes = alloc::vec![0; len];
        let pins_ptr = self.tlb_pins().as_ptr();
        let pins_len = self.tlb_pins().len();
        // Stable emulator pin storage outlives the exclusive memory borrow.
        let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
        let copied = self
            .memory
            .read_ram(pins, addr as u64, &mut bytes)
            .unwrap_or(0);
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

    /// Check if the emulator has been initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
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
    pub fn attach_disk(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
        cylinders: u32,
        heads: u8,
        spt: u8,
    ) -> std::io::Result<()> {
        self.device_manager
            .ide.drives
            .attach_disk(channel, drive, path, cylinders, heads, spt)
    }

    /// Attach a CD-ROM ISO image to a channel/drive (requires std feature)
    #[cfg(feature = "std")]
    pub fn attach_cdrom(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
    ) -> std::io::Result<()> {
        self.device_manager
            .ide.drives
            .attach_cdrom_image(channel, drive, path)
    }

    /// Configure CMOS memory size from total RAM bytes.
    /// This is the preferred method — it matches Bochs devices.cc.
    pub fn configure_memory_in_cmos_from_config(&mut self) {
        self.device_manager
            .cmos
            .set_memory_size_from_bytes(self.config.guest_memory_size as u64);
    }

    /// Configure CMOS memory size (legacy interface)
    pub fn configure_memory_in_cmos(&mut self, base_kb: u16, extended_kb: u16) {
        self.device_manager
            .cmos
            .set_memory_size(base_kb, extended_kb);
    }

    /// Configure CMOS hard drive (type byte only — legacy)
    pub fn configure_disk_in_cmos(&mut self, drive_num: u8, drive_type: u8) {
        self.device_manager
            .cmos
            .set_hard_drive(drive_num, drive_type);
    }

    /// Configure full CMOS hard drive geometry (matching Bochs harddrv.cc)
    pub fn configure_disk_geometry_in_cmos(
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

    /// Configure floppy drives in CMOS
    ///
    /// drive_type: 0=none, 1=360K, 2=1.2M, 3=720K, 4=1.44M, 5=2.88M
    /// Matches Bochs bochsrc `floppya`/`floppyb` type configuration.
    pub fn configure_floppy_in_cmos(&mut self, drive_a_type: u8, drive_b_type: u8) {
        self.device_manager
            .cmos
            .set_floppy_config(drive_a_type, drive_b_type);
    }

    /// Configure boot sequence in CMOS
    ///
    /// Boot device codes: 0=none, 1=floppy, 2=hard disk, 3=cdrom
    pub fn configure_boot_sequence(&mut self, first: u8, second: u8, third: u8) {
        self.device_manager
            .cmos
            .set_boot_sequence(first, second, third);
    }

    #[cfg(feature = "alloc")]
    /// Attach a CD-ROM ISO from in-memory data (for UEFI, WASM, or any environment).
    pub fn attach_cdrom_data(&mut self, channel: usize, drive: usize, data: alloc::vec::Vec<u8>) {
        self.device_manager
            .ide.drives
            .attach_cdrom_data(channel, drive, data);
    }

    #[cfg(feature = "alloc")]
    /// Attach a hard disk from in-memory data (for UEFI, WASM, or any environment).
    ///
    /// Wraps `HardDrive::attach_disk_data()` which stores the disk image
    /// in a `Vec<u8>` instead of using file I/O.
    pub fn attach_disk_data(
        &mut self,
        channel: usize,
        drive: usize,
        data: alloc::vec::Vec<u8>,
        cylinders: u32,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .ide.drives
            .attach_disk_data(channel, drive, data, cylinders, heads, spt);
    }

    /// Attach a CD-ROM ISO from a static byte slice (no-alloc).
    pub fn attach_cdrom_data_ref(&mut self, channel: usize, drive: usize, data: &'static [u8]) {
        self.device_manager
            .ide.drives
            .attach_cdrom_data_ref(channel, drive, data);
    }

    /// Attach a hard disk from a static byte slice (no-alloc).
    pub fn attach_disk_data_ref(
        &mut self,
        channel: usize,
        drive: usize,
        data: &'static [u8],
        cylinders: u32,
        heads: u8,
        spt: u8,
    ) {
        self.device_manager
            .ide.drives
            .attach_disk_data_ref(channel, drive, data, cylinders, heads, spt);
    }

    #[cfg(feature = "alloc")]
    /// Get VGA memory handler probe summary for diagnostics.
    pub fn vga_probe_summary(&self) -> alloc::string::String {
        self.device_manager.vga.probe_summary()
    }

    /// Get the number of registered memory handlers (for diagnostics).
    pub fn memory_handler_count(&self) -> usize {
        self.memory.memory_handler_info()
    }

    /// Get current CS:RIP for diagnostics.
    pub fn get_cs_rip(&self) -> (u16, u64) {
        (self.cpu.get_cs_selector(), self.cpu.rip())
    }

    /// Get CPU mode string for diagnostics.
    pub fn get_cpu_mode_str(&self) -> &'static str {
        match self.cpu.get_cpu_mode() {
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
            self.cpu.lapic_vector_state(vec15)
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
        (self.cpu.activity_state as u32, self.cpu.async_event)
    }

    /// Get CR0 for diagnostics (bit 0 = PE).
    pub fn get_cr0(&self) -> u32 {
        self.cpu.cr0.bits()
    }

    /// Get IF flag for diagnostics.
    pub fn get_if_flag(&self) -> bool {
        self.cpu.get_b_if() != 0
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
        self.cpu.cr3
    }

    /// Get EIP for diagnostics.
    pub fn get_eip(&self) -> u32 {
        self.cpu.eip()
    }

    /// Get segment register info: (selector, base, limit, valid_flags).
    pub fn get_seg_info(&self, seg_idx: usize) -> (u16, u64, u32, u32) {
        if seg_idx < 6 {
            let selector = self.cpu.sregs[seg_idx].selector.value;
            let valid = self.cpu.sregs[seg_idx].cache.valid;
            let base = self.cpu.sregs[seg_idx].cache.u.segment_base();
            let limit = self.cpu.sregs[seg_idx].cache.u.segment_limit_scaled();
            (selector, base, limit, valid)
        } else {
            (0, 0, 0, 0)
        }
    }

    /// Get EAX/EBX/ECX/EDX for diagnostics.
    pub fn get_gpr32(&self, reg: usize) -> u32 {
        match reg {
            0 => self.cpu.eax(),
            1 => self.cpu.ecx(),
            2 => self.cpu.edx(),
            3 => self.cpu.ebx(),
            4 => self.cpu.esp(),
            5 => self.cpu.ebp(),
            6 => self.cpu.esi(),
            7 => self.cpu.edi(),
            _ => 0,
        }
    }

    /// Get the activity state string.
    pub fn get_activity_str(&self) -> &'static str {
        match self.cpu.activity_state {
            CpuActivityState::Active => "active",
            CpuActivityState::Hlt => "hlt",
            CpuActivityState::Shutdown => "shutdown",
            _ => "other",
        }
    }

    /// Get DTLB entry info for a given linear address.
    /// Returns (lpf, ppf, access_bits, host page address) for the TLB slot
    /// that would be used for a dword read at `laddr`.
    pub fn get_dtlb_info(&self, laddr: u64) -> (u64, u64, u32, crate::config::BxPtrEquiv) {
        let idx = self.cpu.dtlb.get_index_of(laddr, 3);
        let entry = &self.cpu.dtlb.entries[idx];
        (
            entry.lpf,
            entry.ppf,
            entry.access_bits,
            crate::cpu::tlb::host_page_addr_bits(self.cpu.mem_host_base, entry.host_page),
        )
    }

    /// Get user_pl flag (true = CPL==3).
    pub fn get_user_pl(&self) -> bool {
        self.cpu.user_pl
    }


    /// Get mem_host_len for diagnostics.
    pub fn get_mem_host_len(&self) -> usize {
        self.cpu.mem_host_len
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
            self.cpu.rip(),
            self.cpu.rsp(),
            self.cpu.rbp()
        );
        tracing::trace!(
            "RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
            self.cpu.rax(),
            self.cpu.rbx(),
            self.cpu.rcx(),
            self.cpu.rdx()
        );
        tracing::trace!(
            "RSI={:#018x} RDI={:#018x} R8={:#018x}  R9={:#018x}",
            self.cpu.rsi(),
            self.cpu.rdi(),
            self.cpu.r8(),
            self.cpu.r9()
        );
        tracing::trace!(
            "CS={:#06x} mode={} IF={}",
            self.cpu.get_cs_selector(),
            self.get_cpu_mode_str(),
            if self.cpu.get_b_if() != 0 { 1 } else { 0 }
        );
        tracing::trace!(
            "CR0={:#010x} CR3={:#018x}",
            self.cpu.cr0.bits(),
            self.cpu.cr3
        );
        tracing::trace!(
            "pending_event={:#010x} event_mask={:#010x} async_event={}",
            self.cpu.pending_event,
            self.cpu.event_mask,
            self.cpu.async_event
        );
        #[cfg(debug_assertions)]
        {
            tracing::trace!(
                "diag: intr_delivered={} if_blocked={} pic_empty={}",
                self.cpu.diag_hae_intr_delivered,
                self.cpu.diag_hae_intr_if_blocked,
                self.cpu.diag_hae_intr_pic_empty
            );
            // SYSCALL ring buffer
            tracing::trace!(
                "--- Last {} SYSCALLs (total={}, sysret={}, blocked={}) ---",
                self.cpu.diag_syscall_ring_idx.min(32),
                self.cpu.diag_syscall_count,
                self.cpu.diag_sysret_count,
                self.cpu
                    .diag_syscall_count
                    .saturating_sub(self.cpu.diag_sysret_count)
            );
            {
                let count = self.cpu.diag_syscall_ring_idx.min(32);
                let start = self.cpu.diag_syscall_ring_idx.saturating_sub(32);
                for i in start..self.cpu.diag_syscall_ring_idx {
                    let (nr, arg0, ic) = self.cpu.diag_syscall_ring[i % 32];
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
            self.cpu.lapic.diag_timer_fires,
            self.cpu.lapic.diag_set_initial_count,
            self.cpu.lapic.diag_timer_masked
        );
        // Show pc_system timer state for LAPIC timer
        if let Some(handle) = self.cpu.lapic.timer_handle {
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
        self.cpu.lapic.dump_state();
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
        let rsp = self.cpu.rsp();
        if rsp > 0xffffffff80000000 {
            let cr3 = self.cpu.cr3 & !0xFFF;
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
                let pdpte = self.read_physical_u64_or_zero(
                    (pml4e & 0xFFFFF_FFFFF000) + pdpt_idx * 8,
                );
                if pdpte & 1 == 0 {
                    return 0;
                }
                if pdpte & 0x80 != 0 {
                    return self.read_physical_u64_or_zero(
                        (pdpte & 0xFFFFF_C0000000) | (addr & 0x3FFFFFFF),
                    );
                }
                let pde = self.read_physical_u64_or_zero(
                    (pdpte & 0xFFFFF_FFFFF000) + pd_idx * 8,
                );
                if pde & 1 == 0 {
                    return 0;
                }
                if pde & 0x80 != 0 {
                    return self.read_physical_u64_or_zero(
                        (pde & 0xFFFFF_FFE00000) | (addr & 0x1FFFFF),
                    );
                }
                let pte = self.read_physical_u64_or_zero(
                    (pde & 0xFFFFF_FFFFF000) + pt_idx * 8,
                );
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
        let rip = self.cpu.rip();
        if rip > 0xffffffff80000000 {
            let cr3 = self.cpu.cr3 & !0xFFF;
            let pml4_idx = (rip >> 39) & 0x1FF;
            let pdpt_idx = (rip >> 30) & 0x1FF;
            let pd_idx = (rip >> 21) & 0x1FF;
            let pt_idx = (rip >> 12) & 0x1FF;
            let pml4e = self.read_physical_u64_or_zero(cr3 + pml4_idx * 8);
            if pml4e & 1 != 0 {
                let pdpte = self.read_physical_u64_or_zero(
                    (pml4e & 0x000FFFFF_FFFFF000) + pdpt_idx * 8,
                );
                if pdpte & 1 != 0 {
                    let paddr = if pdpte & 0x80 != 0 {
                        (pdpte & 0x000FFFFF_C0000000) | (rip & 0x3FFFFFFF)
                    } else {
                        let pde = self.read_physical_u64_or_zero(
                            (pdpte & 0x000FFFFF_FFFFF000) + pd_idx * 8,
                        );
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

// Ensure Emulator is Send (can be moved between threads)
// Each instance is fully independent with no shared state
unsafe impl<T: Instrumentation + Send> Send for Emulator<T> {}

#[cfg(all(test, feature = "alloc"))]
mod tests;
