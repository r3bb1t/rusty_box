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
use crate::iodev::vga::VgaDisplayUpdate;
#[cfg(feature = "alloc")]
use crate::{
    cpu::builder::BxCpuBuilder, iodev::acpi_tables::AcpiTableGenerator, memory::MemoryError,
};
use crate::{
    cpu::{
        apic::{LocalApicCpuEvent, LocalApicTimerActivation, PendingIpi},
        cpu::CpuActivityState,
        instrumentation::{ExitSet, Instrumentation},
        BxCpuC, BxCpuIdTrait, CpuError, CpuidFreq, ResetReason, Result as CpuResult,
    },
    iodev::{
        devices::{DeviceManager, SystemControlPort},
        BxDevicesC, DeviceTimerOwner, TimerRequest,
    },
    memory::{BxMemC, BxMemoryStubC, CpuTlbPin},
    params::BxParams,
    pc_system::{BxPcSystemC, TimerOwner},
    Error, Result,
};

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, format, string::String, sync::Arc, vec::Vec};
#[cfg(not(feature = "alloc"))]
use core::mem::MaybeUninit;
use core::sync::atomic::AtomicBool;
#[cfg(feature = "alloc")]
use core::sync::atomic::Ordering;

// Direct Linux boot (`setup_direct_linux_boot`) is alloc-only.
#[cfg(feature = "alloc")]
const BZIMAGE_MIN_HEADER_LEN: usize = 0x264;
#[cfg(not(feature = "alloc"))]
const NO_ALLOC_MAX_AP_CPUS: usize = (crate::params::BX_MAX_SMP_THREADS_SUPPORTED as usize) - 1;
const BOCHS_APIC_BUS_ID_MASK: u32 = 0xFF;
#[cfg(feature = "alloc")]
const BZIMAGE_BOOT_SIGNATURE_OFFSET: usize = 0x1FE;
#[cfg(feature = "alloc")]
const BZIMAGE_BOOT_SIGNATURE_LO: u8 = 0x55;
#[cfg(feature = "alloc")]
const BZIMAGE_BOOT_SIGNATURE_HI: u8 = 0xAA;
#[cfg(feature = "alloc")]
const BZIMAGE_HEADER_MAGIC_OFFSET: usize = 0x202;
#[cfg(feature = "alloc")]
const BZIMAGE_HEADER_MAGIC: u32 = u32::from_le_bytes(*b"HdrS");
#[cfg(feature = "alloc")]
const BZIMAGE_BOOT_VERSION_OFFSET: usize = 0x206;
#[cfg(feature = "alloc")]
const BZIMAGE_MIN_BOOT_PROTOCOL: u16 = 0x0204;

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

#[cfg(feature = "alloc")]
const DIRECT_MADT_HEADER_SIZE: usize = 44;
#[cfg(feature = "alloc")]
const DIRECT_MADT_LAPIC_ENTRY_SIZE: usize = 8;
#[cfg(feature = "alloc")]
const DIRECT_MADT_IOAPIC_ENTRY_SIZE: usize = 12;
#[cfg(feature = "alloc")]
const DIRECT_MADT_ISO_ENTRY_SIZE: usize = 10;
#[cfg(feature = "alloc")]
const DIRECT_MADT_SIGNATURE: &[u8; 4] = b"APIC";
#[cfg(feature = "alloc")]
const DIRECT_MADT_REVISION: u8 = 3;
#[cfg(feature = "alloc")]
const DIRECT_MADT_OEM_ID: &[u8; 6] = b"RUSTYB";
#[cfg(feature = "alloc")]
const DIRECT_MADT_OEM_TABLE_ID: &[u8; 8] = b"BXMADT  ";
#[cfg(feature = "alloc")]
const DIRECT_MADT_CREATOR_ID: &[u8; 4] = b"RBOX";
#[cfg(feature = "alloc")]
const DIRECT_MADT_REVISION_ID: u32 = 1;
#[cfg(feature = "alloc")]
const DIRECT_MADT_LOCAL_APIC_ADDR: u32 = 0xFEE0_0000;
#[cfg(feature = "alloc")]
const DIRECT_MADT_IOAPIC_ADDR: u32 = 0xFEC0_0000;
#[cfg(feature = "alloc")]
const DIRECT_MADT_PCAT_COMPAT: u32 = 1;
#[cfg(feature = "alloc")]
const DIRECT_MADT_LAPIC_ENABLED: u32 = 1;
#[cfg(feature = "alloc")]
const DIRECT_MADT_IOAPIC_GSI_BASE: u32 = 0;
#[cfg(feature = "alloc")]
const DIRECT_MADT_ISA_BUS: u8 = 0;
#[cfg(feature = "alloc")]
const DIRECT_MADT_TIMER_IRQ: u8 = 0;
#[cfg(feature = "alloc")]
const DIRECT_MADT_TIMER_GSI: u32 = 2;
#[cfg(feature = "alloc")]
const DIRECT_MADT_ISO_CONFORMING_FLAGS: u16 = 0;
#[cfg(feature = "alloc")]
const DIRECT_MADT_ENTRY_TYPE_LAPIC: u8 = 0;
#[cfg(feature = "alloc")]
const DIRECT_MADT_ENTRY_TYPE_IOAPIC: u8 = 1;
#[cfg(feature = "alloc")]
const DIRECT_MADT_ENTRY_TYPE_ISO: u8 = 2;
#[cfg(feature = "alloc")]
const DIRECT_MADT_RESERVED: u8 = 0;
#[cfg(feature = "alloc")]
const ACPI_TABLE_SIGNATURE_OFFSET: usize = 0;
#[cfg(feature = "alloc")]
const ACPI_TABLE_LENGTH_OFFSET: usize = 4;
#[cfg(feature = "alloc")]
const ACPI_TABLE_REVISION_OFFSET: usize = 8;
#[cfg(feature = "alloc")]
const ACPI_TABLE_CHECKSUM_OFFSET: usize = 9;
#[cfg(feature = "alloc")]
const ACPI_TABLE_OEM_ID_OFFSET: usize = 10;
#[cfg(feature = "alloc")]
const ACPI_TABLE_OEM_TABLE_ID_OFFSET: usize = 16;
#[cfg(feature = "alloc")]
const ACPI_TABLE_OEM_REVISION_OFFSET: usize = 24;
#[cfg(feature = "alloc")]
const ACPI_TABLE_CREATOR_ID_OFFSET: usize = 28;
#[cfg(feature = "alloc")]
const ACPI_TABLE_CREATOR_REVISION_OFFSET: usize = 32;
#[cfg(feature = "alloc")]
const DIRECT_MADT_LOCAL_APIC_ADDR_OFFSET: usize = 36;
#[cfg(feature = "alloc")]
const DIRECT_MADT_FLAGS_OFFSET: usize = 40;
#[cfg(feature = "alloc")]
const MADT_ENTRY_TYPE_OFFSET: usize = 0;
#[cfg(feature = "alloc")]
const MADT_ENTRY_LENGTH_OFFSET: usize = 1;
#[cfg(feature = "alloc")]
const MADT_LAPIC_PROCESSOR_ID_OFFSET: usize = 2;
#[cfg(feature = "alloc")]
const MADT_LAPIC_APIC_ID_OFFSET: usize = 3;
#[cfg(feature = "alloc")]
const MADT_LAPIC_FLAGS_OFFSET: usize = 4;
#[cfg(feature = "alloc")]
const MADT_IOAPIC_ID_OFFSET: usize = 2;
#[cfg(feature = "alloc")]
const MADT_IOAPIC_RESERVED_OFFSET: usize = 3;
#[cfg(feature = "alloc")]
const MADT_IOAPIC_ADDR_OFFSET: usize = 4;
#[cfg(feature = "alloc")]
const MADT_IOAPIC_GSI_BASE_OFFSET: usize = 8;
#[cfg(feature = "alloc")]
const MADT_ISO_BUS_OFFSET: usize = 2;
#[cfg(feature = "alloc")]
const MADT_ISO_SOURCE_OFFSET: usize = 3;
#[cfg(feature = "alloc")]
const MADT_ISO_GSI_OFFSET: usize = 4;
#[cfg(feature = "alloc")]
const MADT_ISO_FLAGS_OFFSET: usize = 8;

/// Emulator configuration
#[derive(Debug, Clone)]
pub struct EmulatorConfig {
    /// Guest memory size in bytes
    pub guest_memory_size: usize,
    /// Host memory size in bytes (can be less than guest for swapping)
    pub host_memory_size: usize,
    /// Memory block size for allocation
    pub memory_block_size: usize,
    /// Instructions per second for timing
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
            ips: 4_000_000,
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

#[cfg(feature = "alloc")]
fn build_direct_boot_madt(num_cpus: u32) -> Vec<u8> {
    let madt_len = DIRECT_MADT_HEADER_SIZE
        + (num_cpus as usize * DIRECT_MADT_LAPIC_ENTRY_SIZE)
        + DIRECT_MADT_IOAPIC_ENTRY_SIZE
        + DIRECT_MADT_ISO_ENTRY_SIZE;
    let mut madt = alloc::vec![0u8; madt_len];

    madt[ACPI_TABLE_SIGNATURE_OFFSET..ACPI_TABLE_SIGNATURE_OFFSET + DIRECT_MADT_SIGNATURE.len()]
        .copy_from_slice(DIRECT_MADT_SIGNATURE);
    madt[ACPI_TABLE_LENGTH_OFFSET..ACPI_TABLE_LENGTH_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&(madt_len as u32).to_le_bytes());
    madt[ACPI_TABLE_REVISION_OFFSET] = DIRECT_MADT_REVISION;
    madt[ACPI_TABLE_OEM_ID_OFFSET..ACPI_TABLE_OEM_ID_OFFSET + DIRECT_MADT_OEM_ID.len()]
        .copy_from_slice(DIRECT_MADT_OEM_ID);
    madt[ACPI_TABLE_OEM_TABLE_ID_OFFSET
        ..ACPI_TABLE_OEM_TABLE_ID_OFFSET + DIRECT_MADT_OEM_TABLE_ID.len()]
        .copy_from_slice(DIRECT_MADT_OEM_TABLE_ID);
    madt[ACPI_TABLE_OEM_REVISION_OFFSET
        ..ACPI_TABLE_OEM_REVISION_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_REVISION_ID.to_le_bytes());
    madt[ACPI_TABLE_CREATOR_ID_OFFSET..ACPI_TABLE_CREATOR_ID_OFFSET + DIRECT_MADT_CREATOR_ID.len()]
        .copy_from_slice(DIRECT_MADT_CREATOR_ID);
    madt[ACPI_TABLE_CREATOR_REVISION_OFFSET
        ..ACPI_TABLE_CREATOR_REVISION_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_REVISION_ID.to_le_bytes());
    madt[DIRECT_MADT_LOCAL_APIC_ADDR_OFFSET
        ..DIRECT_MADT_LOCAL_APIC_ADDR_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_LOCAL_APIC_ADDR.to_le_bytes());
    madt[DIRECT_MADT_FLAGS_OFFSET..DIRECT_MADT_FLAGS_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_PCAT_COMPAT.to_le_bytes());

    let mut offset = DIRECT_MADT_HEADER_SIZE;
    for cpu_id in 0..num_cpus {
        madt[offset + MADT_ENTRY_TYPE_OFFSET] = DIRECT_MADT_ENTRY_TYPE_LAPIC;
        madt[offset + MADT_ENTRY_LENGTH_OFFSET] = DIRECT_MADT_LAPIC_ENTRY_SIZE as u8;
        madt[offset + MADT_LAPIC_PROCESSOR_ID_OFFSET] = cpu_id as u8;
        madt[offset + MADT_LAPIC_APIC_ID_OFFSET] = cpu_id as u8;
        madt[offset + MADT_LAPIC_FLAGS_OFFSET
            ..offset + MADT_LAPIC_FLAGS_OFFSET + core::mem::size_of::<u32>()]
            .copy_from_slice(&DIRECT_MADT_LAPIC_ENABLED.to_le_bytes());
        offset += DIRECT_MADT_LAPIC_ENTRY_SIZE;
    }

    madt[offset + MADT_ENTRY_TYPE_OFFSET] = DIRECT_MADT_ENTRY_TYPE_IOAPIC;
    madt[offset + MADT_ENTRY_LENGTH_OFFSET] = DIRECT_MADT_IOAPIC_ENTRY_SIZE as u8;
    madt[offset + MADT_IOAPIC_ID_OFFSET] = num_cpus as u8;
    madt[offset + MADT_IOAPIC_RESERVED_OFFSET] = DIRECT_MADT_RESERVED;
    madt[offset + MADT_IOAPIC_ADDR_OFFSET
        ..offset + MADT_IOAPIC_ADDR_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_IOAPIC_ADDR.to_le_bytes());
    madt[offset + MADT_IOAPIC_GSI_BASE_OFFSET
        ..offset + MADT_IOAPIC_GSI_BASE_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_IOAPIC_GSI_BASE.to_le_bytes());
    offset += DIRECT_MADT_IOAPIC_ENTRY_SIZE;

    madt[offset + MADT_ENTRY_TYPE_OFFSET] = DIRECT_MADT_ENTRY_TYPE_ISO;
    madt[offset + MADT_ENTRY_LENGTH_OFFSET] = DIRECT_MADT_ISO_ENTRY_SIZE as u8;
    madt[offset + MADT_ISO_BUS_OFFSET] = DIRECT_MADT_ISA_BUS;
    madt[offset + MADT_ISO_SOURCE_OFFSET] = DIRECT_MADT_TIMER_IRQ;
    madt[offset + MADT_ISO_GSI_OFFSET..offset + MADT_ISO_GSI_OFFSET + core::mem::size_of::<u32>()]
        .copy_from_slice(&DIRECT_MADT_TIMER_GSI.to_le_bytes());
    madt[offset + MADT_ISO_FLAGS_OFFSET
        ..offset + MADT_ISO_FLAGS_OFFSET + core::mem::size_of::<u16>()]
        .copy_from_slice(&DIRECT_MADT_ISO_CONFORMING_FLAGS.to_le_bytes());

    let sum: u8 = madt.iter().fold(0u8, |a, &b| a.wrapping_add(b));
    madt[ACPI_TABLE_CHECKSUM_OFFSET] = 0u8.wrapping_sub(sum);
    madt
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
/// let mut emu = Emulator::<Corei7SkylakeX>::new(config)?;
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
/// let mut emu = Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
/// let _ = &mut emu.memory;
/// ```
pub struct Emulator<'a, I: BxCpuIdTrait, T: Instrumentation = ()> {
    /// BSP CPU storage. This stays at a stable address for its own cached
    /// host mappings; eviction-visible state instead lives in `cpu_tlb_pins`.
    #[cfg(feature = "alloc")]
    cpu: alloc::boxed::Box<BxCpuC<'a, I, T>>,
    /// Application processors (CPU IDs/APIC IDs 1..N-1).
    #[cfg(feature = "alloc")]
    pub(crate) ap_cpus: Vec<alloc::boxed::Box<BxCpuC<'a, I, T>>>,
    /// Stable descriptors for every CPU's direct host-memory references.
    #[cfg(feature = "alloc")]
    cpu_tlb_pins: Vec<CpuTlbPin>,
    /// BSP CPU storage supplied by no-alloc callers. Its external pin sidecar
    /// is stored separately in the fixed descriptor array below.
    #[cfg(not(feature = "alloc"))]
    cpu: &'a mut BxCpuC<'a, I, T>,
    /// Application processor pointers supplied by no-alloc callers.
    ///
    /// no_std/no-alloc targets can place `BxCpuC` objects in a static, stack,
    /// firmware, or bootloader-provided array and pass those references through
    /// `init_at_with_ap_cpus()`. The emulator stores raw pointers so it does not
    /// need `alloc` or `std` to support SMP scheduling.
    #[cfg(not(feature = "alloc"))]
    ap_cpu_ptrs: [*mut BxCpuC<'a, I, T>; NO_ALLOC_MAX_AP_CPUS],
    #[cfg(not(feature = "alloc"))]
    ap_cpu_count: usize,
    #[cfg(not(feature = "alloc"))]
    cpu_tlb_pins: [MaybeUninit<CpuTlbPin>; NO_ALLOC_MAX_AP_CPUS + 1],
    #[cfg(not(feature = "alloc"))]
    cpu_tlb_pin_count: usize,
    /// Memory subsystem
    pub(crate) memory: BxMemC<'a>,
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
    /// Shared stop flag: when set to true by the GUI thread, run_interactive exits the loop
    #[cfg(feature = "alloc")]
    pub stop_flag: Arc<AtomicBool>,
    #[cfg(not(feature = "alloc"))]
    pub stop_flag: AtomicBool,
}

impl<'a, I: BxCpuIdTrait, T: Instrumentation> Emulator<'a, I, T> {
    /// Extend the borrow of memory owned by this Emulator to match lifetime 'a.
    ///
    /// # Safety
    /// Sound because:
    /// 1. Memory is owned by Emulator which outlives every cpu_loop call
    /// 2. We hold &mut self, preventing concurrent access
    /// 3. CPU does not retain the reference beyond the call
    /// 4. No other code path accesses self.memory during CPU execution
    #[inline]
    unsafe fn borrow_memory_for_cpu(&mut self) -> &'a mut BxMemC<'a> {
        core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(&mut self.memory)
    }

    /// Transmute a `NonNull<BxMemC<'a>>` to `NonNull<BxMemC<'static>>` for wiring
    /// into BxDevicesC during a CPU batch. The pointer remains valid for the
    /// duration of the batch because memory is owned by Emulator.
    ///
    /// # Safety
    /// Caller must ensure the returned pointer is not used after the batch completes.
    #[inline]
    unsafe fn mem_nonnull_static(&mut self) -> core::ptr::NonNull<BxMemC<'static>> {
        core::mem::transmute(core::ptr::NonNull::from(&mut self.memory))
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn cpu_count(&self) -> usize {
        1 + self.ap_cpus.len()
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn cpu_count(&self) -> usize {
        1 + self.ap_cpu_count
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn cpu_ref(&self, index: usize) -> &BxCpuC<'a, I, T> {
        if index == 0 {
            &self.cpu
        } else {
            &self.ap_cpus[index - 1]
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn cpu_ref(&self, index: usize) -> &BxCpuC<'a, I, T> {
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
    #[inline]
    fn raw_tlb_pins(&self) -> &[CpuTlbPin] {
        &self.cpu_tlb_pins
    }

    #[cfg(not(feature = "alloc"))]
    #[inline]
    fn raw_tlb_pins(&self) -> &[CpuTlbPin] {
        // SAFETY: `init_at_with_ap_cpus` initializes precisely this prefix
        // before exposing the emulator, and CPU storage outlives it.
        unsafe {
            core::slice::from_raw_parts(
                self.cpu_tlb_pins.as_ptr().cast::<CpuTlbPin>(),
                self.cpu_tlb_pin_count,
            )
        }
    }

    /// Return the stable all-CPU pin slice after synchronizing any CPU state
    /// changed outside a wired scope.
    #[inline]
    pub(crate) fn tlb_pins(&self) -> &[CpuTlbPin] {
        self.refresh_dirty_tlb_pins();
        self.raw_tlb_pins()
    }
    /// Refresh every stable external pin sidecar before a CPU/device memory
    /// scope. This happens while CPUs are only shared-borrowed; afterwards the
    /// running CPU updates its own sidecar synchronously on every mapping
    /// install or invalidation.
    fn refresh_tlb_pins(&self) {
        let pins = self.raw_tlb_pins();
        debug_assert_eq!(pins.len(), self.cpu_count());
        for (index, pin) in pins.iter().enumerate() {
            self.cpu_ref(index).refresh_tlb_pin(pin);
        }
    }

    /// Refresh only sidecars dirtied while their CPU was not wired.
    #[inline]
    fn refresh_dirty_tlb_pins(&self) {
        let pins = self.raw_tlb_pins();
        debug_assert_eq!(pins.len(), self.cpu_count());
        for (index, pin) in pins.iter().enumerate() {
            self.cpu_ref(index).refresh_tlb_pin_if_dirty(pin);
        }
    }
    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    fn total_cpu_icount(&self) -> u64 {
        (0..self.cpu_count()).fold(0u64, |total, cpu_index| {
            total.saturating_add(self.cpu_ref(cpu_index).icount)
        })
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<'a, I, T> {
        if index == 0 {
            &mut self.cpu
        } else {
            &mut self.ap_cpus[index - 1]
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<'a, I, T> {
        if index == 0 {
            self.cpu
        } else {
            assert!(index <= self.ap_cpu_count);
            // SAFETY: &mut self guarantees no other emulator method can
            // concurrently borrow the AP CPU through this pointer.
            unsafe { &mut *self.ap_cpu_ptrs[index - 1] }
        }
    }

    /// Invalidate every host pointer and decoded trace before memory backing
    /// can be replaced or restored.
    pub(crate) fn invalidate_all_cpu_host_mappings(&mut self) {
        self.clear_scheduler_raw_wiring();
        let smc_seq = self.memory.smc_seq_next();
        for cpu_index in 0..self.cpu_count() {
            let cpu = self.cpu_mut_at(cpu_index);
            cpu.invalidate_host_memory_mappings();
            // A full icache flush consumes every memory-side SMC event,
            // including a snapshot restore that restarted the sequence.
            cpu.smc_seq_seen = smc_seq;
        }
        self.refresh_tlb_pins();
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
                self.device_manager.harddrv.pending_seek_arm_usec[channel][device].is_some()
                    || self.device_manager.harddrv.seek_timer_handles[channel][device]
                        .is_some_and(|handle| self.pc_system.is_timer_active(handle))
            });
            if seek_in_flight {
                continue;
            }
            if pic.irq_line_level(irq) != self.device_manager.harddrv.get_irq_level(channel)
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

    fn cpu_runnable_for_batch(&self, index: usize) -> bool {
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
                        & (BxCpuC::<I>::BX_EVENT_PENDING_INTR
                            | BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR))
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
    fn refresh_cpu_masks(&mut self, index: usize) {
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
    fn assert_cpu_masks_match_scan(&self) {
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

    /// Clear all raw device-side wiring before touching machine-owned state.
    ///
    /// CPU wrappers clear their own memory/I/O/pc-system buses. The device
    /// manager pointers are installed only around an individual CPU slice, so
    /// every scheduler commit runs with ordinary exclusive borrows.
    fn clear_scheduler_raw_wiring(&mut self) {
        self.devices.clear_device_manager();
        self.device_manager.mem_ptr = None;
        self.device_manager.active_tlb_pins = None;
        self.device_manager.active_tlb_pin_count = 0;
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
    fn ap_fast_forward_allowed(runnable_mask: CpuMask, cpu_count: usize) -> bool {
        runnable_mask.next_set(1, cpu_count).is_none()
    }

    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    fn can_fast_forward_bsp_hlt(&self) -> bool {
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

    /// Run a CPU batch with full I/O wiring.
    ///
    /// Sets up all the NonNull pointers that `cpu_loop_n_with_io` needs,
    /// executes `batch_size` instructions, then tears down the wiring.
    ///
    /// # Safety
    /// Same invariants as `borrow_memory_for_cpu`: caller must hold `&mut self`
    /// and no other code path may access memory/devices during the batch.
    pub unsafe fn run_cpu_batch(&mut self, batch_size: u64) -> CpuResult<u64> {
        unsafe { self.run_cpu_batch_with_strict_limit(batch_size, false) }
    }

    unsafe fn run_cpu_batch_with_strict_limit(
        &mut self,
        batch_size: u64,
        strict_limit: bool,
    ) -> CpuResult<u64> {
        if self.snapshot_restore_failed {
            return Err(CpuError::CpuNotInitialized);
        }
        self.batch_advanced_pc_system = false;
        self.clear_scheduler_raw_wiring();
        // A reset applied at batch entry needs no special handling: the batch
        // below simply starts executing at the reset vector.
        self.service_scheduler_boundary(0)?;
        self.refresh_dirty_tlb_pins();

        let cpu_count = self.cpu_count();
        let smp = cpu_count > 1;
        let pins_ptr = self.tlb_pins().as_ptr();
        let pins_len = self.tlb_pins().len();
        let mem_ptr: *mut BxMemC<'a> = &mut self.memory;
        let io_ptr = core::ptr::NonNull::from(&mut self.devices);
        let ps_ptr = core::ptr::NonNull::from(&mut self.pc_system);
        let dm_ptr = core::ptr::NonNull::from(&mut self.device_manager);
        let pic_ref: *mut _ = &mut self.device_manager.pic;
        let dma_ref: *mut _ = &mut self.device_manager.dma;
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

                let mem_static = self.mem_nonnull_static();
                (*io_ptr.as_ptr()).set_device_manager(dm_ptr);
                (*dm_ptr.as_ptr()).mem_ptr = Some(mem_static);
                (*dm_ptr.as_ptr()).active_tlb_pins =
                    core::ptr::NonNull::new(pins_ptr as *mut CpuTlbPin);
                (*dm_ptr.as_ptr()).active_tlb_pin_count = pins_len;

                let mem_extended: &'a mut BxMemC<'a> =
                    core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(&mut *mem_ptr);
                let pins = core::slice::from_raw_parts(pins_ptr, pins_len);
                let current_pin = &*pins_ptr.add(cpu_index);
                let ticks_before = self.cpu_ref(cpu_index).cpu_ticks();
                let slice_result = if smp {
                    self.cpu_mut_at(cpu_index).cpu_run_trace_with_io(
                        mem_extended,
                        pins,
                        current_pin,
                        per_cpu_batch,
                        strict_smp_deadline,
                        cpu_count as u64,
                        io_ptr,
                        ps_ptr,
                        Some(&mut *pic_ref),
                        Some(&mut *dma_ref),
                    )
                } else {
                    self.cpu_mut_at(cpu_index).cpu_loop_n_with_io(
                        mem_extended,
                        pins,
                        current_pin,
                        per_cpu_batch,
                        strict_up_deadline,
                        1,
                        io_ptr,
                        ps_ptr,
                        Some(&mut *pic_ref),
                        Some(&mut *dma_ref),
                    )
                };

                // CPU wrappers clear their own buses. Clear every device-side
                // raw pointer before consuming any queued machine work.
                self.clear_scheduler_raw_wiring();
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

        self.clear_scheduler_raw_wiring();
        #[cfg(test)]
        self.assert_cpu_masks_match_scan();
        result.map(|_| {
            if smp {
                total_elapsed_ticks
            } else {
                total_up_executed
            }
        })
    }

    /// Bochs `cpu: quantum=N` (BXPN_SMP_QUANTUM), clamped to config.h
    /// BX_SMP_QUANTUM_MIN..=BX_SMP_QUANTUM_MAX.
    #[inline]
    fn smp_quantum_ticks(&self) -> u64 {
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
        let mem_ptr: *const BxMemC<'a> = &self.memory;
        for cpu_index in 0..self.cpu_count() {
            if self.cpu_ref(cpu_index).smc_seq_seen < newest {
                // SAFETY: memory and the cpu array are distinct fields of
                // self; smc_apply_pending only reads the pending queue.
                unsafe {
                    self.cpu_mut_at(cpu_index)
                        .smc_apply_pending(&*mem_ptr, true)
                };
            }
        }
        self.memory.smc_clear_pending();
    }

    /// Inject an external interrupt with temporary memory bus wiring.
    ///
    /// Wires the memory bus so the interrupt path can read IVT/IDT and push
    /// stack frames, then clears it after injection.
    ///
    /// Used by `run_interactive` / `step_batch` for manual interrupt delivery
    /// between CPU batches. Also available for no-alloc callers doing their
    /// own batch loops (e.g. UEFI example).
    ///
    /// # Safety
    /// Same invariants as `borrow_memory_for_cpu`.
    pub unsafe fn inject_interrupt(&mut self, vector: u8) -> CpuResult<()> {
        self.refresh_tlb_pins();
        let pins_ptr = self.tlb_pins().as_ptr();
        let pins_len = self.tlb_pins().len();
        let mem_extended = self.borrow_memory_for_cpu();
        let pins = core::slice::from_raw_parts(pins_ptr, pins_len);
        self.cpu.wire_memory_access(
            core::ptr::NonNull::from(&mut *mem_extended),
            pins,
            &*pins_ptr,
        );
        let result = self.cpu.inject_external_interrupt(vector);
        self.cpu.clear_memory_access();
        self.refresh_cpu_masks(0);
        result
    }
}

#[cfg(feature = "alloc")]
impl<'a, I: BxCpuIdTrait> Emulator<'a, I, ()> {
    /// Create a new emulator with no instrumentation (`T = ()`).
    ///
    /// Returns `Box<Self>` because Emulator is ~1.4 MB.
    pub fn new(config: EmulatorConfig) -> Result<Box<Self>> {
        Self::new_inner(config, || Ok(BxCpuBuilder::<I>::new().build()?))
    }
}

#[cfg(feature = "alloc")]
impl<'a, I: BxCpuIdTrait, T: Instrumentation> Emulator<'a, I, T> {
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
        let mut cpu = BxCpuBuilder::<I>::new().build_with_tracer(tracer)?;
        cpu.configure_smp(0, topology);
        cpu.set_smp_quantum(config.smp_quantum);
        cpu.set_cpuid_freq(config.cpuid_freq, config.ips);
        Self::new_from_parts(config, cpu, Vec::new())
    }

    fn new_inner<F>(config: EmulatorConfig, mut build_cpu: F) -> Result<Box<Self>>
    where
        F: FnMut() -> Result<alloc::boxed::Box<BxCpuC<'static, I, T>>>,
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
        cpu: alloc::boxed::Box<BxCpuC<'static, I, T>>,
        ap_cpus: Vec<alloc::boxed::Box<BxCpuC<'static, I, T>>>,
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
            core::ptr::addr_of_mut!((*ptr).stop_flag).write(Arc::new(AtomicBool::new(false)));
            Ok(alloc::boxed::Box::from_raw(ptr))
        }
    }
}

impl<'a, I: BxCpuIdTrait, T: Instrumentation> Emulator<'a, I, T> {
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
        cpu: &'a mut BxCpuC<'a, I, T>,
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
        cpu: &'a mut BxCpuC<'a, I, T>,
        ap_cpus: &mut [&'a mut BxCpuC<'a, I, T>],
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
            let ap_cpu: &mut BxCpuC<'a, I, T> = &mut **ap_cpu_slot;
            ap_cpu.configure_smp((index + 1) as u32, topology);
            ap_cpu.set_smp_quantum(config.smp_quantum);
            ap_cpu.set_cpuid_freq(config.cpuid_freq, config.ips);
            ap_cpu_ptrs[index] = ap_cpu as *mut BxCpuC<'a, I, T>;
        }
        // The descriptor sidecars are 40 KiB each. Initialize only the used
        // prefix directly in the caller-provided Emulator storage so no-alloc
        // construction neither allocates nor builds/moves a 254-entry stack
        // temporary. Sidecar addresses become stable before any CPU scope
        // wires one into `active_tlb_pin_sidecar`.
        let bsp_ptr = cpu as *mut BxCpuC<'a, I, T>;
        core::ptr::addr_of_mut!((*ptr).cpu).write(cpu);
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
        core::ptr::addr_of_mut!((*ptr).stop_flag).write(AtomicBool::new(false));
        Ok(&mut *ptr)
    }

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
    fn register_timer_owners(&mut self) -> Result<()> {
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
    pub fn cpu(&self) -> &BxCpuC<'a, I, T> {
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
    pub(crate) fn cpu_mut(&mut self) -> &mut BxCpuC<'a, I, T> {
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
    /// let mut first = Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
    /// let mut second = Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
    /// // Safe code cannot obtain mutable CPU storage to swap it.
    /// core::mem::swap(first.cpu_mut(), second.cpu_mut());
    /// ```
    #[inline]
    pub unsafe fn cpu_mut_unchecked(&mut self) -> &mut BxCpuC<'a, I, T> {
        self.cpu_mut_at(0)
    }

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

    fn rearm_device_timers_after_hardware_reset(&mut self) {
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
            .harddrv
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
            .harddrv
            .attach_cdrom_image(channel, drive, path)
    }

    /// Check if an interrupt is pending (PIC or LAPIC)
    pub fn has_interrupt(&self) -> bool {
        // Legacy PIC path
        if self.device_manager.has_interrupt() {
            return true;
        }
        // APIC path: check LAPIC for pending interrupts
        if self.cpu.lapic_has_intr() {
            return true;
        }
        false
    }

    /// Acknowledge interrupt and get vector
    pub fn iac(&mut self) -> u8 {
        self.device_manager.iac()
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
    fn pump_gui_input(&mut self) {}

    #[cfg(feature = "alloc")]
    fn pump_gui_input(&mut self) {
        let mut scancodes_to_send = Vec::new();
        let mut mouse_to_send = Vec::new();
        let mut serial_input = Vec::new();
        if let Some(gui) = &mut self.gui {
            gui.handle_events();
            scancodes_to_send = gui.get_pending_scancodes();
            mouse_to_send = gui.get_pending_mouse();
            serial_input = gui.get_pending_serial_input();
        }
        let keyboard_changed = !scancodes_to_send.is_empty() || !mouse_to_send.is_empty();
        let serial_changed = !serial_input.is_empty();
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
            if let Some(delay) = self.device_manager.serial.take_fifo_timer_update(0) {
                self.devices.request_timer_after_usec(
                    DeviceTimerOwner::SerialFifo(0),
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
        // A reset applied here needs no branch: host input reaches a machine
        // that resumes at the reset vector either way.
        if let Err(error) = self.service_scheduler_boundary(0) {
            tracing::error!("host-input scheduler boundary failed: {error:?}");
        }
    }

    #[inline]
    fn advance_pc_system_after_cpu_ticks(&mut self, ticks: u64) {
        // On reset the boundary discards `ticks` itself; execution resumes at
        // the reset vector, so both outcomes continue identically here.
        if let Err(error) = self.service_scheduler_boundary(ticks) {
            tracing::error!("scheduler tick commit failed: {error:?}");
        }
    }

    /// Advance a fully halted machine directly to its earliest exact timer
    /// deadline. Host input is pumped before each halted step.
    #[inline]
    fn hlt_wait_step_ticks(&self) -> u32 {
        self.pc_system.get_num_cpu_ticks_left_next_event().max(1)
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

    fn drain_lapic_bus(&mut self) {
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
                .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
    fn service_lapic_timer_requests(&mut self) {
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

    fn service_lapic_local_events(&mut self) {
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
                        cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
                    cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
                self.device_manager.ioapic.receive_eoi(vector);
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
            || self.device_manager.ioapic.num_pending_deliveries != 0
            // PIC edge bookkeeping awaiting collapse to a final level.
            || self.device_manager.pic.irq_pending
            || self.device_manager.pic.irq_cleared
            // PIC→IOAPIC edge forwards (drained by dispatch_timer_fires).
            || self.device_manager.pic.num_ioapic_forwards != 0
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
        self.clear_scheduler_raw_wiring();

        // Bochs unmapped.cc port 0x8900: a completed "Shutdown" protocol sets
        // `bx_user_quit = 1` and BX_FATALs. Translate that guest request into
        // our run-loop stop flag (checked at the top of every batch) — a
        // graceful stop at the next boundary in place of Bochs's immediate
        // abort. Drained unconditionally (the flag lives on `devices`, outside
        // `scheduler_boundary_work_pending`'s DeviceManager view).
        if self.devices.take_shutdown_request() {
            tracing::info!("port 0x8900 shutdown protocol complete — stopping emulation");
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
            eprintln!(
                "machine boundary failed to quiesce: pending={:?}",
                (
                    self.device_manager.pci_ide_bar4_needs_reregister,
                    self.device_manager.acpi_pm_needs_reregister,
                    self.device_manager.acpi_sm_needs_reregister,
                    self.device_manager.pam_needs_update,
                    self.device_manager.smram_needs_update,
                    self.device_manager.bios_write_needs_update,
                    self.device_manager.vga_bar_needs_reregister,
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

            let now = self.pc_system.time_ticks();
            let until_deadline = self
                .pc_system
                .next_timer_deadline_ticks()
                .map(|deadline| deadline.saturating_sub(now).max(1))
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

    /// Apply the side effects the HPET queued from MMIO context — the calls
    /// Bochs hpet.cc performs synchronously inside its handlers
    /// (`update_irq`, `activate_timer_nsec`, `deactivate_timer`,
    /// `DEV_pit_enable_irq`, `DEV_cmos_enable_irq`, `DEV_MEM_WRITE_PHYSICAL`).
    /// Comparator deadlines were pre-anchored at the access instant, so the
    /// drain point does not shift them.
    fn drain_hpet_pending(&mut self) {
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
                // GSI 16..23 exist only as IOAPIC pins here. DELIBERATE Bochs
                // deviation, flagged not silent: Bochs update_irq() routes
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
                crate::memory::CpuMemoryPolicy::default(),
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

    fn apply_lapic_cpu_event(&mut self, target: usize, event: Option<LocalApicCpuEvent>) {
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
            LocalApicCpuEvent::Sipi(vector) => {
                // deliver_sipi VMexits when the target is in VMX non-root
                // operation, and the exit can walk the VMEXIT MSR store/load
                // lists. Wire the memory bus for the call so those guest-memory
                // accesses resolve, then clear it (mirrors inject_interrupt).
                self.refresh_tlb_pins();
                let pins_ptr = self.tlb_pins().as_ptr();
                let pins_len = self.tlb_pins().len();
                let mem_ptr =
                    core::ptr::NonNull::from(&mut *unsafe { self.borrow_memory_for_cpu() });
                let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
                let current_pin = unsafe { &*pins_ptr.add(target) };
                let cpu = self.cpu_mut_at(target);
                cpu.wire_memory_access(mem_ptr, pins, current_pin);
                cpu.deliver_sipi(vector);
                cpu.clear_memory_access();
            }
        }
    }
    /// Rebuild CPU interrupt-level bits after snapshot restore without
    /// consuming any restored PIC, IOAPIC, LAPIC, or timer work queues.
    #[cfg(feature = "std")]
    fn sync_restored_event_levels(&mut self) {
        let pic_asserted = self.device_manager.pic.has_interrupt()
            || self.device_manager.pic.irq_pending
            || self.pc_system.intr_raised;
        if pic_asserted {
            self.cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
        } else {
            self.cpu.clear_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
        }

        for cpu_index in 0..self.cpu_count() {
            let cpu = self.cpu_mut_at(cpu_index);
            if cpu.lapic.intr || cpu.lapic.intr_pending {
                cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
            } else {
                cpu.clear_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
            }
        }
    }

    /// Synchronize final physical interrupt levels after all queue owners have
    /// committed. This is deliberately not a scheduler entry point.
    fn sync_final_event_levels(&mut self) {
        // Publish the physical PIC pin on every commit, not only when legacy
        // edge bookkeeping happens to be present. This restores the level
        // after CPU reset and prevents lost interrupt state.
        let asserted = self.device_manager.pic.has_interrupt();
        self.device_manager.pic.irq_pending = false;
        self.device_manager.pic.irq_cleared = false;
        if asserted {
            self.cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
        } else {
            self.cpu.clear_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
        }

        // PIC forwarding has already changed IOAPIC levels; now route its
        // deferred deliveries in registration order into the LAPICs.
        let (deliveries, count) = self.device_manager.ioapic.take_pending_deliveries();
        for &delivery in &deliveries[..count] {
            let mut delivery = delivery;
            if delivery.needs_pic_iac {
                delivery.vector = self.device_manager.pic.iac();
                delivery.needs_pic_iac = false;
            }
            let done = self.deliver_ioapic_to_lapics(delivery);
            self.device_manager
                .ioapic
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
                cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
                cpu.lapic.intr_pending = false;
            }
            self.refresh_cpu_masks(cpu_index);
        }

        if self.pc_system.intr_raised {
            self.cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
            self.pc_system.intr_raised = false;
        }
        if self.pc_system.intr_cleared {
            self.cpu.clear_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
            self.pc_system.intr_cleared = false;
        }
        if self.pc_system.async_event_pending {
            self.cpu.async_event = 1;
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
    /// * 0x11000: command line (up to 2048 bytes)
    /// * 0x100000: protected-mode kernel
    /// * High memory: initramfs (if provided)
    pub fn setup_direct_linux_boot(
        &mut self,
        bzimage: &[u8],
        initramfs: Option<&[u8]>,
        cmdline: &str,
    ) -> Result<()> {
        // Validate bzImage header
        if bzimage.len() < BZIMAGE_MIN_HEADER_LEN {
            return Err(Error::Cpu(CpuError::InvalidBootImage {
                reason: "bzImage too small",
            }));
        }
        if bzimage[BZIMAGE_BOOT_SIGNATURE_OFFSET] != BZIMAGE_BOOT_SIGNATURE_LO
            || bzimage[BZIMAGE_BOOT_SIGNATURE_OFFSET + 1] != BZIMAGE_BOOT_SIGNATURE_HI
        {
            return Err(Error::Cpu(CpuError::InvalidBootImage {
                reason: "Invalid bzImage boot signature",
            }));
        }
        let header_magic = u32::from_le_bytes([
            bzimage[BZIMAGE_HEADER_MAGIC_OFFSET],
            bzimage[BZIMAGE_HEADER_MAGIC_OFFSET + 1],
            bzimage[BZIMAGE_HEADER_MAGIC_OFFSET + 2],
            bzimage[BZIMAGE_HEADER_MAGIC_OFFSET + 3],
        ]);
        if header_magic != BZIMAGE_HEADER_MAGIC {
            return Err(Error::Cpu(CpuError::InvalidBootImage {
                reason: "Invalid bzImage header magic",
            }));
        }
        let boot_version = u16::from_le_bytes([
            bzimage[BZIMAGE_BOOT_VERSION_OFFSET],
            bzimage[BZIMAGE_BOOT_VERSION_OFFSET + 1],
        ]);
        if boot_version < BZIMAGE_MIN_BOOT_PROTOCOL {
            return Err(Error::Cpu(CpuError::InvalidBootImage {
                reason: "boot protocol too old (need >= 2.04)",
            }));
        }

        // Parse bzImage header
        let setup_sects = if bzimage[0x1F1] == 0 {
            4
        } else {
            bzimage[0x1F1] as usize
        };
        let setup_size = (setup_sects + 1) * 512;
        let pm_kernel = &bzimage[setup_size..];

        let code32_start = u32::from_le_bytes([
            bzimage[0x214],
            bzimage[0x215],
            bzimage[0x216],
            bzimage[0x217],
        ]);

        // Read pref_address (protocol >= 2.10) and init_size for boot_params placement
        let pref_address = if boot_version >= 0x020A {
            u64::from_le_bytes([
                bzimage[0x258],
                bzimage[0x259],
                bzimage[0x25A],
                bzimage[0x25B],
                bzimage[0x25C],
                bzimage[0x25D],
                bzimage[0x25E],
                bzimage[0x25F],
            ])
        } else {
            0 // Old kernels: use legacy boot_params address
        };
        let init_size = u32::from_le_bytes([
            bzimage[0x260],
            bzimage[0x261],
            bzimage[0x262],
            bzimage[0x263],
        ]) as u64;

        tracing::debug!(
            "bzImage: protocol {}.{}, setup={}B, kernel={}B, entry={:#x}, pref={:#x}, init_size={:#x}",
            boot_version >> 8, boot_version & 0xFF,
            setup_size, pm_kernel.len(), code32_start, pref_address, init_size
        );

        // =====================================================================
        // Write GDT at 0x1000
        // =====================================================================
        const GDT_ADDR: u64 = 0x1000;
        let gdt: [u64; 4] = [
            0x0000000000000000, // Entry 0: null
            0x0000000000000000, // Entry 1: null (reserved)
            0x00CF9A000000FFFF, // Entry 2 (sel 0x10): 32-bit code, base=0, limit=4GB
            0x00CF92000000FFFF, // Entry 3 (sel 0x18): 32-bit data, base=0, limit=4GB
        ];
        let mut gdt_bytes = [0u8; 32];
        for (i, &entry) in gdt.iter().enumerate() {
            gdt_bytes[i * 8..(i + 1) * 8].copy_from_slice(&entry.to_le_bytes());
        }
        self.load_ram(&gdt_bytes, GDT_ADDR)?;

        // =====================================================================
        // Write boot_params (zero page)
        // =====================================================================
        // Place boot_params at 0x10000 (standard location, matches QEMU).
        // The decompressor relocates itself to ~pref_address+init_size area,
        // which would overwrite boot_params if placed there. Low addresses
        // (< 0x100000) are safe — the compressed kernel loads at 0x100000+
        // and the decompressor never touches conventional memory.
        // The kernel's early page fault handler (__early_make_pgtable) creates
        // identity mappings on demand for any unmapped physical address.
        let boot_params_addr: u64 = 0x10000;
        let cmdline_addr: u64 = 0x20000;
        tracing::debug!(
            "boot_params at {:#x}, cmdline at {:#x} (pref={:#x}, init_size={:#x})",
            boot_params_addr,
            cmdline_addr,
            pref_address,
            init_size
        );
        let mut boot_params = [0u8; 4096];

        // Copy setup header from bzImage (offsets 0x1F1 to 0x268)
        let hdr_start = 0x1F1;
        let hdr_end = core::cmp::min(0x268, bzimage.len());
        boot_params[hdr_start..hdr_end].copy_from_slice(&bzimage[hdr_start..hdr_end]);

        // type_of_loader = 0xFF (unknown bootloader)
        boot_params[0x210] = 0xFF;

        // loadflags: set LOADED_HIGH (bit 0), keep CAN_USE_HEAP (bit 7)
        boot_params[0x211] |= 0x01; // LOADED_HIGH

        // cmd_line_ptr = physical address of command line
        boot_params[0x228..0x22C].copy_from_slice(&(cmdline_addr as u32).to_le_bytes());

        // heap_end_ptr: relative to setup header start (unused for direct boot)
        boot_params[0x224..0x226].copy_from_slice(&0xFE00u16.to_le_bytes());

        // screen_info (struct screen_info at boot_params offset 0x000):
        //   0x00: orig_x           (cursor column)
        //   0x01: orig_y           (cursor row)
        //   0x02: ext_mem_k        (u16, extended memory in KB)
        //   0x04: orig_video_page  (u16, active display page)
        //   0x06: orig_video_mode  (video mode number)
        //   0x07: orig_video_cols  (text columns)
        //   0x0a: orig_video_ega_bx (u16, EGA/VGA info)
        //   0x0e: orig_video_lines (text rows)
        //   0x0f: orig_video_isVGA (0=no, 1=VGA, 0x22=EGA/VGA)
        //   0x10: orig_video_points (u16, font height in pixels)
        boot_params[0x00] = 0; // orig_x
        boot_params[0x01] = 0; // orig_y
        boot_params[0x06] = 0x03; // orig_video_mode = 3 (80x25 color text)
        boot_params[0x07] = 80; // orig_video_cols
        boot_params[0x0E] = 25; // orig_video_lines
        boot_params[0x0F] = 0x01; // orig_video_isVGA = 1
        boot_params[0x10..0x12].copy_from_slice(&16u16.to_le_bytes()); // orig_video_points = 16

        // vid_mode at 0x1FA (in setup header, but also used by kernel)
        boot_params[0x1FA..0x1FC].copy_from_slice(&0xFFFFu16.to_le_bytes()); // NORMAL_VGA

        // acpi_rsdp_addr at offset 0x070 (boot protocol 2.14+)
        // Tells kernel where to find RSDP without scanning BIOS area
        boot_params[0x070..0x078].copy_from_slice(&0x40000u64.to_le_bytes());

        // =====================================================================
        // Set up initramfs if provided
        // =====================================================================
        let kernel_end = code32_start as u64 + pm_kernel.len() as u64;

        // initrd_addr_max from boot protocol (offset 0x22C) - max address kernel can handle
        let initrd_addr_max = if boot_version >= 0x0203 {
            u32::from_le_bytes([
                bzimage[0x22C],
                bzimage[0x22D],
                bzimage[0x22E],
                bzimage[0x22F],
            ]) as u64
        } else {
            0x37FFFFFF // Default for old protocols
        };

        if let Some(initrd_data) = initramfs {
            let ram_top = self.config.guest_memory_size as u64;
            let max_addr = core::cmp::min(ram_top, initrd_addr_max + 1);

            // Place initramfs at top of allowed memory (QEMU strategy)
            // This prevents the kernel decompressor from overwriting the initramfs
            let initrd_load_addr = (max_addr - initrd_data.len() as u64) & !0xFFF;

            tracing::debug!(
                "BOOT LAYOUT: kernel={} bytes at {:#x}..{:#x}, init_size={:#x}, initrd={} bytes at {:#x}..{:#x}, RAM top={:#x}, initrd_addr_max={:#x}",
                pm_kernel.len(), code32_start, kernel_end,
                init_size,
                initrd_data.len(), initrd_load_addr, initrd_load_addr + initrd_data.len() as u64,
                ram_top, initrd_addr_max
            );
            self.load_ram(initrd_data, initrd_load_addr)?;

            // ramdisk_image = physical address
            boot_params[0x218..0x21C].copy_from_slice(&(initrd_load_addr as u32).to_le_bytes());
            // ramdisk_size
            boot_params[0x21C..0x220].copy_from_slice(&(initrd_data.len() as u32).to_le_bytes());
        }

        // =====================================================================
        // E820 memory map
        // =====================================================================
        let ram_size = self.config.guest_memory_size as u64;
        let e820_base = 0x2D0; // offset in boot_params
        let mut e820_idx = 0;

        // Helper to write an e820 entry (20 bytes each)
        let mut write_e820 = |bp: &mut [u8], addr: u64, size: u64, etype: u32| {
            let off = e820_base + e820_idx * 20;
            bp[off..off + 8].copy_from_slice(&addr.to_le_bytes());
            bp[off + 8..off + 16].copy_from_slice(&size.to_le_bytes());
            bp[off + 16..off + 20].copy_from_slice(&etype.to_le_bytes());
            e820_idx += 1;
        };

        // Entry 1: 0 - 0x9FC00 (conventional memory, ~639KB)
        write_e820(&mut boot_params, 0, 0x9FC00, 1);
        // Entry 2: 0x9FC00 - 0xA0000 (reserved, EBDA)
        write_e820(&mut boot_params, 0x9FC00, 0x400, 2);
        // Entry 3: 0xF0000 - 0x100000 (reserved, BIOS)
        write_e820(&mut boot_params, 0xF0000, 0x10000, 2);
        // Entry 4: 0x100000 - top of RAM (usable extended memory)
        if ram_size > 0x100000 {
            write_e820(&mut boot_params, 0x100000, ram_size - 0x100000, 1);
        }

        // e820_entries count at offset 0x1E8
        boot_params[0x1E8] = e820_idx as u8;

        // Write boot_params to memory
        self.load_ram(&boot_params, boot_params_addr)?;

        // =====================================================================
        // Write command line
        // =====================================================================
        let cmdline_bytes = cmdline.as_bytes();
        let cmdline_len = core::cmp::min(cmdline_bytes.len(), 2047);
        let mut cmdline_buf = alloc::vec![0u8; cmdline_len + 1]; // null-terminated
        cmdline_buf[..cmdline_len].copy_from_slice(&cmdline_bytes[..cmdline_len]);
        self.load_ram(&cmdline_buf, cmdline_addr)?;
        tracing::debug!("Command line: {}", cmdline);

        // =====================================================================
        // Create minimal ACPI tables (RSDP → XSDT → MADT)
        // Without these, the kernel can't find the APIC/IOAPIC and falls back
        // to a mode where no interrupt delivery works, stalling boot.
        // Layout: RSDP at 0xE0000, XSDT at 0xE0100, MADT at 0xE0200
        // =====================================================================
        {
            // Place in low memory (safe area: 0x40000-0x4FFFF unused by kernel/bootloader)
            const RSDP_ADDR: u64 = 0x40000;
            const XSDT_ADDR: u64 = 0x40100;
            const MADT_ADDR: u64 = 0x40200;

            let madt = build_direct_boot_madt(self.config.cpu_params.cpu_count());
            let madt_len = madt.len();
            self.load_ram(&madt, MADT_ADDR)?;

            // --- XSDT (Extended System Description Table) ---
            // Header: 36 bytes + 1 pointer (8 bytes) = 44 bytes
            let xsdt_len: u32 = 36 + 8;
            let mut xsdt = alloc::vec![0u8; xsdt_len as usize];
            xsdt[0..4].copy_from_slice(b"XSDT");
            xsdt[4..8].copy_from_slice(&xsdt_len.to_le_bytes());
            xsdt[8] = 1; // revision
            xsdt[10..16].copy_from_slice(b"RUSTYB");
            xsdt[16..24].copy_from_slice(b"BXXSDT  ");
            xsdt[24..28].copy_from_slice(&1u32.to_le_bytes());
            xsdt[28..32].copy_from_slice(b"RBOX");
            xsdt[32..36].copy_from_slice(&1u32.to_le_bytes());
            // Pointer to MADT (64-bit)
            xsdt[36..44].copy_from_slice(&MADT_ADDR.to_le_bytes());
            let sum: u8 = xsdt.iter().fold(0u8, |a, &b| a.wrapping_add(b));
            xsdt[9] = 0u8.wrapping_sub(sum);
            self.load_ram(&xsdt, XSDT_ADDR)?;

            // --- RSDP (Root System Description Pointer) ---
            // RSDP v2.0 = 36 bytes
            let mut rsdp = [0u8; 36];
            rsdp[0..8].copy_from_slice(b"RSD PTR "); // signature
                                                     // checksum (byte 8) — filled later
            rsdp[9..15].copy_from_slice(b"RUSTYB"); // OEM ID
            rsdp[15] = 2; // revision (2 = ACPI 2.0+)
                          // RSDT address (offset 16) — point to XSDT address as 32-bit for v1 compat
            rsdp[16..20].copy_from_slice(&(XSDT_ADDR as u32).to_le_bytes());
            // Length (offset 20) — v2.0 extended length
            rsdp[20..24].copy_from_slice(&36u32.to_le_bytes());
            // XSDT address (offset 24) — 64-bit
            rsdp[24..32].copy_from_slice(&XSDT_ADDR.to_le_bytes());
            // Extended checksum (byte 32) — filled later
            // v1 checksum covers bytes 0-19
            let v1_sum: u8 = rsdp[0..20].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            rsdp[8] = 0u8.wrapping_sub(v1_sum);
            // v2 extended checksum covers bytes 0-35
            let v2_sum: u8 = rsdp.iter().fold(0u8, |a, &b| a.wrapping_add(b));
            rsdp[32] = 0u8.wrapping_sub(v2_sum);
            self.load_ram(&rsdp, RSDP_ADDR)?;

            tracing::debug!(
                "ACPI tables: RSDP at {:#x}, XSDT at {:#x}, MADT at {:#x} ({}B)",
                RSDP_ADDR,
                XSDT_ADDR,
                MADT_ADDR,
                madt_len
            );
        }

        // =====================================================================
        // Initialize PIC and PIT (normally done by BIOS POST)
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

        // =====================================================================
        // Load protected-mode kernel at code32_start
        // =====================================================================
        tracing::debug!(
            "Loading kernel ({} bytes) at {:#x}",
            pm_kernel.len(),
            code32_start
        );
        self.load_ram(pm_kernel, code32_start as u64)?;

        // =====================================================================
        // Configure CPU for protected mode
        // =====================================================================
        self.cpu.setup_for_direct_boot(GDT_ADDR);

        // Set entry point and registers
        self.cpu.set_rip(code32_start as u64);
        self.cpu.set_rsp(0x20000); // Temporary stack (kernel sets its own early)
        self.cpu.set_rsi(boot_params_addr); // ESI = pointer to boot_params

        tracing::debug!(
            "Direct boot ready: EIP={:#x}, ESI={:#x}, ESP={:#x}",
            code32_start,
            boot_params_addr,
            0x20000u32
        );

        Ok(())
    }

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
    pub fn run_interactive(&mut self, max_instructions: u64) -> Result<u64>
    where
        'a: 'static, // Required for borrow_memory_for_cpu safety
    {
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
        while instructions_executed < max_instructions && !self.stop_flag.load(Ordering::Relaxed) {
            // 1. Handle GUI events (keyboard/mouse/serial input) first.
            self.pump_gui_input();

            // 2. Execute CPU instructions in batches
            let remaining_instructions = max_instructions - instructions_executed;
            let batch_size = remaining_instructions.min(INSTRUCTION_BATCH_SIZE);
            // SAFETY: see borrow_memory_for_cpu / run_cpu_batch
            let result =
                unsafe { self.run_cpu_batch_with_strict_limit(batch_size, true) };


            let _should_update_gui = match result {
                Ok(executed) => {
                    instructions_executed += executed;

                    // Reset HLT+IF=0 counter on any non-zero batch
                    if executed > 0 {
                        hlt_if0_count = 0;
                    }

                    // Milestone progress print every 500K instructions
                    #[cfg(debug_assertions)]
                    if instructions_executed % 500_000 < INSTRUCTION_BATCH_SIZE {
                        tracing::trace!(
                            "[{}k instr] RIP={:#010x} CS={:#06x} mode={} batch_returned={} activity={:?}",
                            instructions_executed / 1000,
                            self.cpu.rip(),
                            self.cpu.get_cs_selector(),
                            self.cpu.get_cpu_mode(),
                            executed,
                            self.cpu.activity_state,
                        );
                    }
                    // Detect zero-return batches (HLT or stuck)
                    if executed == 0 {
                        // HLT with IF=0: CPU is dead (panic or intentional halt)
                        // Use counter-based approach: only break after N consecutive
                        // zero-batch HLT+IF=0 cycles. This allows transient IF=0 states
                        // (e.g. kernel cli/hlt sequences before init scripts) to recover.
                        if matches!(
                            self.cpu.activity_state,
                            CpuActivityState::Hlt
                                | CpuActivityState::Mwait
                                | CpuActivityState::MwaitIf
                        ) && !self.cpu.interrupts_enabled()
                        {
                            hlt_if0_count += 1;
                            // Warn once at 1000 but DON'T break — match egui behavior.
                            // The egui path never exits on HLT+IF=0 and eventually the
                            // kernel recovers (timer/NMI wakes CPU). Breaking here would
                            // prevent headless Alpine from reaching modloop phase.
                            if hlt_if0_count == 1000 {
                                tracing::trace!(
                                    "[ZERO-BATCH] HLT/MWAIT with IF=0 for 1000 consecutive batches at RIP={:#x} CS={:#06x} activity={:?} — continuing (egui-match)",
                                    self.cpu.rip(), self.cpu.get_cs_selector(), self.cpu.activity_state,
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
                                let _ = writeln!(f, "{}", msg);
                            }
                        }
                        tracing::warn!("{}", msg);
                    }

                    if self.cpu.is_in_shutdown() {
                        #[cfg(feature = "std")]
                        log_reset(&format!(
                            "TRIPLE-FAULT SHUTDOWN at RIP={:#x} CS={:#06x} icount={}",
                            self.cpu.rip(),
                            self.cpu.get_cs_selector(),
                            self.cpu.icount
                        ));
                        break;
                    }


                    // -- Progress tracking --
                    let current_rip = self.cpu.rip();

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
                            self.cpu.get_cs_selector(),
                            current_rip,
                            self.cpu.get_cpu_mode(),
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
                                let bp = self.cpu.bp() as usize;
                                let ss_base = self.cpu.get_ss_base() as usize;
                                let bp_phys = ss_base + bp;
                                let ax = self.cpu.eax() as u16;
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
                                    self.cpu.get_cs_selector(),
                                    self.cpu.get_cpu_mode(),
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
                        // Drain E9 output to prevent buffer growth (output discarded)
                        let _ = self.devices.take_port_e9_output();
                    }

                    // Advance virtual time (Bochs-like ticking).
                    // Required so PIT can generate IRQ0 and BIOS can progress past HLT waits.
                    if self.config.ips != 0 {
                        if matches!(
                            self.cpu.activity_state,
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
                                matches!(self.cpu.activity_state, CpuActivityState::MwaitIf);
                            let mut hlt_budget = 0u64;
                            while hlt_budget < 100_000_000 {
                                // Service host input while halted so an idle
                                // (tickless) guest wakes promptly on a keypress,
                                // instead of stalling for the whole halt budget.
                                self.pump_gui_input();
                                if self.has_interrupt()
                                    && (self.cpu.interrupts_enabled() || mwait_if)
                                {
                                    break;
                                }
                                if self.stop_flag.load(core::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                self.service_lapic_local_events();
                                if self.cpu.lapic.intr
                                    && (self.cpu.interrupts_enabled() || mwait_if)
                                {
                                    self.cpu
                                        .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
                            if self.cpu.lapic_has_intr() {
                                self.cpu
                                    .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
                                    && self.cpu.get_b_if() != 0
                                    && !self.cpu.interrupts_inhibited(0x01)
                                {
                                    let vec = self.iac();
                                    // SAFETY: see borrow_memory_for_cpu / inject_interrupt
                                    if let Err(e) = unsafe { self.inject_interrupt(vec) } {
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
                                // SAFETY: see borrow_memory_for_cpu / run_cpu_batch
                                let r2 = unsafe {
                                    self.run_cpu_batch_with_strict_limit(batch2, true)
                                };
                                if let Ok(ex2) = r2 {
                                    instructions_executed += ex2;
                                    if !self.batch_advanced_pc_system {
                                        self.advance_pc_system_after_cpu_ticks(ex2);
                                    }
                                } else {
                                    break;
                                }
                                // If CPU re-entered MWAIT, advance time again
                                if !matches!(
                                    self.cpu.activity_state,
                                    CpuActivityState::Hlt
                                        | CpuActivityState::Mwait
                                        | CpuActivityState::MwaitIf
                                ) {
                                    break; // CPU is active — return to outer loop
                                }
                                // HLT loop: Bochs handleWaitForEvent advances BX_TICKN(10).
                                let mwait_if2 =
                                    matches!(self.cpu.activity_state, CpuActivityState::MwaitIf);
                                let mut hlt2 = 0u64;
                                while hlt2 < 100_000_000 {
                                    // Keep host input responsive during MWAIT idle.
                                    self.pump_gui_input();
                                    if self.has_interrupt()
                                        && (self.cpu.interrupts_enabled() || mwait_if2)
                                    {
                                        break;
                                    }
                                    self.service_lapic_local_events();
                                    if self.cpu.lapic.intr
                                        && (self.cpu.interrupts_enabled() || mwait_if2)
                                    {
                                        self.cpu
                                            .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
                                if self.cpu.lapic_has_intr() {
                                    self.cpu
                                        .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
                                }
                            }
                        }
                    }

                    // Drive pc_system timers via Bochs-exact tickn() mechanism.
                    if !self.batch_advanced_pc_system {
                        self.advance_pc_system_after_cpu_ticks(executed);
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
                            executed, instructions_executed / 1000, self.cpu.rip(), pit_c0_count,
                            self.cpu.activity_state, bda_ticks);
                    }

                    // Periodic interrupt-chain diagnostic (every ~1M instructions)
                    #[cfg(debug_assertions)]
                    if instructions_executed % 1_000_000 < INSTRUCTION_BATCH_SIZE {
                        let has_int = self.has_interrupt();
                        let if_flag = self.cpu.get_b_if();
                        let rip = self.cpu.rip();
                        let pit_c0 = &self.device_manager.pit.counters[0];
                        tracing::trace!(
                            "IRQ-DIAG: {}M instr, RIP={:#x}, IF={}, has_int={}, PIC_imr={:#04x}, PIC_irr={:#04x}, PIT_c0: mode={:?} inlatch={} count={} count_written={} gate={} output={}",
                            instructions_executed / 1_000_000,
                            rip,
                            if_flag,
                            has_int,
                            self.device_manager.pic.master.imr,
                            self.device_manager.pic.master.irr,
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
                        && self.cpu.get_b_if() != 0
                        && !self.cpu.interrupts_inhibited(0x01)
                    // BX_INHIBIT_INTERRUPTS
                    {
                        let vector = self.iac();

                        // Temporarily wire the memory bus so the interrupt path can
                        // read IVT/IDT and push stack frames correctly.
                        // SAFETY: see borrow_memory_for_cpu / inject_interrupt
                        let inject_result = unsafe { self.inject_interrupt(vector) };

                        match &inject_result {
                            Ok(()) => {
                                tracing::trace!(
                                    "INT-INJECT: OK! activity_after={:?}, RIP={:#x}",
                                    self.cpu.activity_state,
                                    self.cpu.rip()
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
                        let _ = std::io::stdout().write_all(serial_bytes.as_slice());
                        let _ = std::io::stdout().flush();
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
                        let rip = self.cpu.rip();
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
                        self.cpu.rip(),
                        self.cpu.get_cs_selector(),
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
                let pi = self.cpu.perf_instructions;
                let tlb_h = self.cpu.perf_tlb_hit;
                let tlb_m = self.cpu.perf_tlb_miss;
                let pw = self.cpu.perf_page_walk;
                let ic_m = self.cpu.perf_icache_miss;
                let pf = self.cpu.perf_prefetch;
                let tlb_total = tlb_h + tlb_m;
                let tlb_pct = if tlb_total > 0 {
                    tlb_h as f64 / tlb_total as f64 * 100.0
                } else {
                    0.0
                };
                // cpu_ticks = instruction count plus the fast-REP tick surplus
                // (Bochs BX_TICK1-per-instruction + BX_TICKN time domain).
                let bochs_ticks = self.cpu.cpu_ticks();
                tracing::debug!("[PERF] dispatches={pi} bochs_ticks={bochs_ticks} tlb_hit={tlb_h} tlb_miss={tlb_m} tlb_hit%={tlb_pct:.2}% page_walks={pw}");
            }
        }

        Ok(instructions_executed)
    }

    /// Execute a batch of instructions cooperatively (no blocking loop).
    ///
    /// Designed for single-threaded environments like WASM or UEFI where the
    /// caller must yield control back to its event loop regularly. Runs up to
    /// `max_instructions`, ticks devices, syncs A20, then returns.
    ///
    /// Returns `(instructions_executed, is_shutdown)`.
    pub fn step_batch(&mut self, max_instructions: u64) -> Result<(u64, bool)>
    where
        'a: 'static,
    {
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
            let result = unsafe { self.run_cpu_batch(max_instructions) };

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
                            .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
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
                        .signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
                }
            }

            // --- Deliver PIC interrupt ---
            if self.device_manager.has_interrupt()
                && self.cpu.get_b_if() != 0
                && !self.cpu.interrupts_inhibited(0x01)
            {
                let vector = self.iac();
                // SAFETY: see borrow_memory_for_cpu / inject_interrupt
                if let Err(e) = unsafe { self.inject_interrupt(vector) } {
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
    /// Attach a CD-ROM ISO from in-memory data (for UEFI, WASM, or any environment).
    pub fn attach_cdrom_data(&mut self, channel: usize, drive: usize, data: alloc::vec::Vec<u8>) {
        self.device_manager
            .harddrv
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
            .harddrv
            .attach_disk_data(channel, drive, data, cylinders, heads, spt);
    }

    /// Attach a CD-ROM ISO from a static byte slice (no-alloc).
    pub fn attach_cdrom_data_ref(&mut self, channel: usize, drive: usize, data: &'static [u8]) {
        self.device_manager
            .harddrv
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
            .harddrv
            .attach_disk_data_ref(channel, drive, data, cylinders, heads, spt);
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
        let ch1 = &self.device_manager.harddrv.channels[1];
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
    /// Returns (lpf, ppf, access_bits, host_page_addr) for the TLB slot
    /// that would be used for a dword read at `laddr`.
    pub fn get_dtlb_info(&self, laddr: u64) -> (u64, u64, u32, crate::config::BxPtrEquiv) {
        let idx = self.cpu.dtlb.get_index_of(laddr, 3);
        let entry = &self.cpu.dtlb.entries[idx];
        (
            entry.lpf,
            entry.ppf,
            entry.access_bits,
            entry.host_page_addr,
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

impl<I: BxCpuIdTrait, T: Instrumentation> Emulator<'_, I, T> {
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
            self.device_manager.diag_pit_fires,
            self.device_manager.diag_irq0_latched,
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
        let hist: Vec<(u8, u8, u32)> = self.device_manager.harddrv.cmd_history.iter().collect();
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
unsafe impl<I: BxCpuIdTrait + Send, T: Instrumentation + Send> Send for Emulator<'_, I, T> {}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::cpu::core_i7_skylake::Corei7SkylakeX;
    use crate::cpu::decoder::Instruction;
    use crate::cpu::{
        instrumentation::{CpuSetupMode, X86Reg},
        rusty_box::MemoryAccessType,
    };
    use crate::memory::CpuMemoryPolicy;
    use crate::pc_system::TimerOwner;
    const TEST_SMP_PACKAGES: u32 = 2;
    const TEST_SMP_CORES: u32 = 1;
    const TEST_SMP_THREADS: u32 = 1;
    const BSP_INDEX: usize = 0;
    const AP_INDEX: usize = 1;
    const AP_TRAMPOLINE_VECTOR: u8 = 0x08;
    const AP_TRAMPOLINE_ADDR: u64 = (AP_TRAMPOLINE_VECTOR as u64) << 12;
    const AP_TRAMPOLINE_OPCODE: u8 = 0x90;
    const AP_TRAMPOLINE_LEN: usize = 32;
    const AP_BATCH_INSTRUCTIONS: u64 = 16;
    const DIRECT_BOOT_MADT_TEST_CPUS: u32 = 3;
    const UNSET_APIC_ID: u8 = 0xFF;
    const ACPI_CHECKSUM_VALID_SUM: u8 = 0;
    const EXPECTED_DIRECT_BOOT_LAPIC_IDS: [u8; DIRECT_BOOT_MADT_TEST_CPUS as usize] = [0, 1, 2];
    const TEST_LAPIC_TIMER_VECTOR: u32 = 0x40;
    const LVT_TIMER_PERIODIC_MODE: u32 = 1 << 17;
    const TEST_LAPIC_TIMER_PERIOD_TICKS: u64 = 10;
    const TEST_LAPIC_TIMER_ELAPSED_TICKS: u32 = 50;
    const FW_CFG_IO_BASE: u16 = 0x510;
    const FW_CFG_DATA_PORT: u16 = 0x511;
    const FW_CFG_NB_CPUS_KEY: u16 = 0x05;
    const FW_CFG_MAX_CPUS_KEY: u16 = 0x0F;
    const FW_CFG_SELECTOR_WRITE_BYTES: u8 = 2;
    const FW_CFG_DATA_READ_BYTES: u8 = 1;
    const NONFLAT_TOPOLOGY_CPUS: u32 = 8;
    const MAX_SUPPORTED_TEST_CPUS: u32 = 254;
    const CPUID_LEAF_FEATURE_INFO: u32 = 0x0000_0001;
    const CPUID_LEAF_EXTENDED_TOPOLOGY: u32 = 0x0000_000B;
    const CPUID_LEAF1_LOGICAL_COUNT_SHIFT: u32 = 16;
    const CPUID_LEAF1_APIC_ID_SHIFT: u32 = 24;
    const CPUID_APIC_ID_BYTE_MASK: u32 = 0xFF;
    const CPUID_TOPOLOGY_SUBLEAF_SMT: u32 = 0;
    const CPUID_TOPOLOGY_SUBLEAF_CORE: u32 = 1;
    const CPUID_TOPOLOGY_SUBLEAF_PACKAGE: u32 = 2;
    const CPUID_TOPOLOGY_LEVEL_TYPE_SHIFT: u32 = 8;
    const CPUID_TOPOLOGY_LEVEL_TYPE_SMT: u32 = 1;
    const CPUID_TOPOLOGY_LEVEL_TYPE_CORE: u32 = 2;

    fn resident_host_base(emu: &mut Emulator<'_, Corei7SkylakeX>) -> *mut u8 {
        let pins_ptr = emu.tlb_pins().as_ptr();
        let pins_len = emu.tlb_pins().len();
        // Stable CPU pin storage outlives the exclusive memory borrow.
        let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
        emu.memory
            .get_host_mem_addr_pinned(
                0,
                MemoryAccessType::RW,
                pins,
                CpuMemoryPolicy::default(),
            )
            .unwrap()
            .expect("resident block must have a pinned direct span")
            .as_mut_ptr()
    }

    fn topology_level_ecx(subleaf: u32, level_type: u32) -> u32 {
        subleaf | (level_type << CPUID_TOPOLOGY_LEVEL_TYPE_SHIFT)
    }

    const ICR_LOW: u64 = 0x300;
    const ICR_HIGH: u64 = 0x310;
    const ICR_TARGET_AP: u32 = 1;
    const ICR_LEVEL_ASSERT: u32 = 1 << 14;
    const ICR_TRIGGER_LEVEL: u32 = 1 << 15;

    fn send_bsp_icr_init(emu: &mut Emulator<'_, Corei7SkylakeX, ()>) {
        let bsp = emu.cpu_mut_at(BSP_INDEX);
        bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24, 0);
        bsp.lapic.write_aligned(ICR_LOW, ((crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8)
            | ICR_LEVEL_ASSERT
            | ICR_TRIGGER_LEVEL, 0);
        bsp.lapic.write_aligned(ICR_LOW, (crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8 | ICR_TRIGGER_LEVEL, 0);
        emu.refresh_cpu_masks(BSP_INDEX);
    }

    fn send_bsp_icr_sipi(emu: &mut Emulator<'_, Corei7SkylakeX, ()>, vector: u8) {
        let bsp = emu.cpu_mut_at(BSP_INDEX);
        bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24, 0);
        bsp.lapic.write_aligned(ICR_LOW, vector as u32
            | ((crate::cpu::apic::ApicDeliveryMode::Sipi as u32) << 8)
            | ICR_LEVEL_ASSERT, 0);
        emu.refresh_cpu_masks(BSP_INDEX);
    }

    fn read_fw_cfg_u16(fw_cfg: &mut crate::iodev::fw_cfg::BxFwCfg, key: u16) -> u16 {
        fw_cfg.write_port(
            FW_CFG_IO_BASE,
            key as u32,
            FW_CFG_SELECTOR_WRITE_BYTES,
            None,
            &[],
        );
        let lo = fw_cfg.read_port_mut(FW_CFG_DATA_PORT, FW_CFG_DATA_READ_BYTES) as u16;
        let hi = fw_cfg.read_port_mut(FW_CFG_DATA_PORT, FW_CFG_DATA_READ_BYTES) as u16;
        lo | (hi << 8)
    }

    fn acpi_madt_from_tables(tables: &[u8]) -> &[u8] {
        let offset = tables
            .windows(DIRECT_MADT_SIGNATURE.len())
            .position(|window| window == DIRECT_MADT_SIGNATURE)
            .expect("MADT signature missing from ACPI tables");
        let len = u32::from_le_bytes(
            tables[offset + ACPI_TABLE_LENGTH_OFFSET
                ..offset + ACPI_TABLE_LENGTH_OFFSET + core::mem::size_of::<u32>()]
                .try_into()
                .unwrap(),
        ) as usize;
        &tables[offset..offset + len]
    }

    fn parse_madt_ids<const N: usize>(madt: &[u8]) -> ([u8; N], usize, Option<u8>) {
        let mut offset = DIRECT_MADT_HEADER_SIZE;
        let mut lapic_ids = [UNSET_APIC_ID; N];
        let mut lapic_count = 0usize;
        let mut ioapic_id = None;

        while offset < madt.len() {
            let entry_type = madt[offset + MADT_ENTRY_TYPE_OFFSET];
            let entry_len = madt[offset + MADT_ENTRY_LENGTH_OFFSET] as usize;
            match entry_type {
                DIRECT_MADT_ENTRY_TYPE_LAPIC => {
                    if lapic_count < N {
                        lapic_ids[lapic_count] = madt[offset + MADT_LAPIC_APIC_ID_OFFSET];
                    }
                    lapic_count += 1;
                }
                DIRECT_MADT_ENTRY_TYPE_IOAPIC => {
                    ioapic_id = Some(madt[offset + MADT_IOAPIC_ID_OFFSET]);
                }
                _ => {}
            }
            offset += entry_len;
        }

        (lapic_ids, lapic_count, ioapic_id)
    }

    #[test]
    fn test_emulator_creation() {
        // BxICache contains ~19MB fixed arrays; debug-mode struct literal needs large stack
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                #[cfg(feature = "std")]
                assert!(emu.bios_output_file.is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn instrumented_constructor_applies_configured_cpuid_frequency() {
        struct NoopTracer;
        impl crate::cpu::instrumentation::Instrumentation for NoopTracer {}

        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                let mut config = EmulatorConfig::default();
                config.cpuid_freq = crate::cpu::cpuid::CpuidFreq::Ips;
                config.ips = 120_000_000;
                let mut emu =
                    Emulator::<Corei7SkylakeX, NoopTracer>::new_with_mode_and_instrumentation(
                        config,
                        CpuSetupMode::FlatProtected32,
                        NoopTracer,
                    )
                    .unwrap();
                emu.virt_write(CODE_ADDR, &[0x0F, 0xA2, 0xEB, 0xFE])
                    .unwrap();
                emu.reg_write(X86Reg::Rax, 0x15);
                emu.reg_write(X86Reg::Rcx, 0);

                emu.emu_start(CODE_ADDR, Some(CODE_ADDR + 2), None, Some(8))
                    .unwrap();

                assert_eq!(emu.reg_read(X86Reg::Rax), 1);
                assert_eq!(emu.reg_read(X86Reg::Rbx), 1);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 120_000_000);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_emulator_initialization() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                assert!(!emu.is_initialized());

                let result = emu.initialize();
                assert!(result.is_ok());
                assert!(emu.is_initialized());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_multiple_instances_independent() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();

                let mut emu1 = Emulator::<Corei7SkylakeX>::new(config.clone()).unwrap();
                let emu2 = Emulator::<Corei7SkylakeX>::new(config).unwrap();

                emu1.initialize().unwrap();

                // emu2 should still be uninitialized
                assert!(emu1.is_initialized());
                assert!(!emu2.is_initialized());

                // Different tick counts
                emu1.pc_system.tickn(1000);
                assert_eq!(emu1.ticks(), 1000);
                assert_eq!(emu2.ticks(), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn strict_smp_deadline_keeps_bochs_idle_cpu_quantum_credit() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[0x90; 32], 0x1000).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let quantum = emu.smp_quantum_ticks();
                assert!(quantum > 1);
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, 1, true, false, "one_tick")
                    .unwrap();

                let elapsed = unsafe { emu.run_cpu_batch(quantum / 2) }.unwrap();

                assert_eq!(elapsed, (1 + quantum) / 2);
                assert_eq!(emu.pc_system.time_ticks(), elapsed);
                assert_eq!(emu.smp_tick_remainder, (1 + quantum) % 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn equal_smp_deadline_truncates_a_short_final_round() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[0x90; 32], 0x1000).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let quantum = emu.smp_quantum_ticks();
                let short_round = quantum / 2;
                assert!(short_round > 1);
                let _ = unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.pc_system.initialize(1_000_000);
                emu.pc_system
                    .register_timer(
                        TimerOwner::NullTimer,
                        short_round,
                        false,
                        true,
                        "equal_short_round",
                    )
                    .unwrap();
                let icount_before = emu.cpu_ref(BSP_INDEX).icount;

                let elapsed = unsafe { emu.run_cpu_batch(short_round) }.unwrap();

                assert_eq!(
                    emu.cpu_ref(BSP_INDEX).icount - icount_before,
                    short_round,
                    "an equal deadline must make the shortened round strict"
                );
                assert_eq!(elapsed, (short_round + quantum) / 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn strict_budget_stops_a_linked_branch_chain_exactly() {
        // A hot dec/jnz loop links its back edge (Bochs cpu.cc linkTrace);
        // the strict instruction budget must stop the linked chain at exactly
        // the requested count — the link guard's `iteration < max` is the
        // batch-capped UP form of linkTrace's ticks-left guard.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                const CODE: u64 = 0x1000;
                emu.reg_write(X86Reg::Rcx, 1_000_000);
                // dec ecx; jnz -3
                emu.virt_write(CODE, &[0x49, 0x75, 0xFD]).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);

                let executed =
                    unsafe { emu.run_cpu_batch_with_strict_limit(501, true) }.unwrap();

                assert_eq!(executed, 501, "strict budget must be exact");
                // 501 instructions = 251 decs + 250 taken jnz.
                assert_eq!(emu.reg_read(X86Reg::Rcx), 1_000_000 - 251);
                // The loop is hot enough that the back edge must have linked;
                // the budget stop above therefore covers the linked path.
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_stops_at_the_next_exact_timer_deadline() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = [0x90u8; 8192];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, 1, true, true, "one_tick")
                    .unwrap();

                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();
                assert!(
                    (1..128).contains(&executed),
                    "one-tick deadline did not stop the active batch promptly: {executed}"
                );
                assert_eq!(emu.pc_system.time_ticks(), executed);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smi_apm_handshake_runs_the_guest_smm_handler() {
        // The Bochs BIOS smm_init contract (rombios32.c): outb(0xb3, 1),
        // outb(0xb2, 0) raises an SMI (APMC_EN set via ACPI config 0x58 bit
        // 25); the CPU enters SMM at SMBASE+0x8000 = 0x38000 and the GUEST
        // handler clears 0xb3 and RSMs. POST then polls 0xb3 until 0.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // BIOS smm_init: enable SMI generation on APMC writes.
                emu.device_manager.acpi.pci_write(0x58, 1 << 25, 4);

                // Guest code at 0x1000: apms := 1, then the SMI command.
                //   mov al, 1 ; out 0xb3, al ; mov al, 0 ; out 0xb2, al ; nops
                let mut code = [0x90u8; 64];
                code[..8].copy_from_slice(&[0xB0, 0x01, 0xE6, 0xB3, 0xB0, 0x00, 0xE6, 0xB2]);
                emu.virt_write(0x1000, &code).unwrap();
                // SMM handler at 0x38000 (SMBASE 0x30000 + entry 0x8000):
                //   mov al, 0 ; out 0xb3, al ; rsm
                emu.virt_write(0x38000, &[0xB0, 0x00, 0xE6, 0xB3, 0x0F, 0xAA])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                // Run: OUT 0xB2 ends the slice (machine boundary), the
                // boundary delivers the SMI to CPU 0, the next batch enters
                // SMM, runs the handler, and RSM resumes the interrupted code.
                // NOTE: sampling `smm_mode()` at batch boundaries cannot observe
                // the SMM visit — entry, handler and RSM all complete inside a
                // single batch, so the flag is already false at every sample.
                // The deterministic proof that SMM ran is `apms == 0` below:
                // only the handler at 0x38000 issues `out 0xb3, 0`, and that
                // address is reachable only via SMI entry at SMBASE+0x8000.
                for _ in 0..8 {
                    let _ = unsafe { emu.run_cpu_batch(64) }.unwrap();
                    emu.service_scheduler_boundary(0).unwrap();
                    if emu.device_manager.pci2isa.apms == 0
                        && !emu.cpu_mut_at(0).smm_mode()
                        && emu.reg_read(X86Reg::Rip) > 0x1008
                    {
                        break;
                    }
                }

                assert_eq!(
                    emu.device_manager.pci2isa.apms, 0,
                    "the guest SMM handler must clear apms (out 0xb3, 0) — this is \
                     the proof that the SMI was delivered and SMM was entered, since \
                     the handler at 0x38000 is only reachable via SMBASE+0x8000"
                );
                assert!(
                    !emu.cpu_mut_at(0).smm_mode(),
                    "RSM must have exited System Management Mode"
                );
                // Execution resumed past the OUT 0xB2 that raised the SMI (the
                // interrupted instruction stream continues; where it stops among
                // the trailing nops is irrelevant).
                assert!(
                    emu.reg_read(X86Reg::Rip) > 0x1008,
                    "execution must resume after the OUT that raised the SMI"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pit_generates_periodic_irq0_across_multiple_periods() {
        // Linux check_timer() needs the 8254 PIT to deliver *repeated* IRQ0
        // ticks. Program counter 0 in mode 2 (rate generator) and confirm the
        // owner keeps firing, producing many IRQ0 rising edges — not one.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000); // 1 tick = 1 microsecond
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // Counter 0, LSB/MSB, mode 2 (rate generator), binary: 0x34.
                // Divisor 100 → ~84 us period.
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_CONTROL, 0x34, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 100, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 0, 1, 0);

                let now = emu.pc_system.time_ticks();
                let delay = emu.device_manager.pit.next_event_usec();
                assert!(delay.is_some(), "programmed PIT must have a periodic deadline");
                emu.devices
                    .request_timer_after_usec(DeviceTimerOwner::Pit, now, delay);
                emu.drain_device_timer_requests();

                let before = emu.device_manager.diag_pit_fires;
                // Advance 2 ms; a ~84 us period should fire ~23 times.
                emu.service_scheduler_boundary(2_000).unwrap();
                let fires = emu.device_manager.diag_pit_fires - before;
                assert!(
                    fires >= 5,
                    "PIT mode 2 must generate repeated IRQ0 edges, got {fires}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn atapi_seek_timer_completes_the_read_at_the_exact_deadline() {
        // Bochs harddrv.cc start_seek/seek_timer: an ATAPI READ arms a
        // distance-proportional seek timer; DRQ and the channel IRQ appear
        // only when pc_system reaches that deadline — never before.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                use crate::iodev::harddrv::AtaStatus;

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000); // 1 tick = 1 microsecond
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // 4-sector disc: media init parks curr_lba at 3; READ(10) of
                // LBA 0 seeks |0 - 3 + 1| / 4 of the 80 ms stroke = 40000 us.
                emu.device_manager
                    .harddrv
                    .attach_cdrom_data(0, 0, vec![0u8; 2048 * 4]);
                {
                    let crate::iodev::devices::DeviceManager {
                        harddrv,
                        pic,
                        pci_ide,
                        ..
                    } = &mut emu.device_manager;
                    harddrv.write(0x1f7, 0xA0, 1, pic, pci_ide); // PACKET
                    let packet = [0x28u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
                    for word in packet.chunks_exact(2) {
                        let value = u16::from_le_bytes([word[0], word[1]]) as u32;
                        harddrv.write(0x1f0, value, 2, pic, pci_ide);
                    }
                }
                // The I/O layer drains the arm right after the OUT dispatch
                // (iodev/mod.rs) — mirror that contract here.
                let now = emu.pc_system.time_ticks();
                let arm = emu.device_manager.harddrv.take_pending_seek_arm(0, 0);
                assert_eq!(arm, Some(40_000));
                emu.devices.request_timer_after_usec(
                    DeviceTimerOwner::HdSeek(0),
                    now,
                    arm.map(u64::from),
                );
                emu.drain_device_timer_requests();

                // One tick before the deadline: still seeking, no DRQ, no IRQ.
                emu.service_scheduler_boundary(39_999).unwrap();
                let drive = &emu.device_manager.harddrv.channels[0].drives[0];
                assert!(!drive.controller.status.contains(AtaStatus::DRQ));
                assert!(!drive.controller.interrupt_pending);
                assert!(!emu.device_manager.pic.irq_line_level(14));

                // Crossing the deadline completes the command.
                emu.service_scheduler_boundary(2).unwrap();
                let drive = &emu.device_manager.harddrv.channels[0].drives[0];
                assert!(drive.controller.status.contains(AtaStatus::DRQ));
                assert!(drive.controller.interrupt_pending);
                assert!(emu.device_manager.pic.irq_line_level(14));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pit_irq0_delivers_through_ioapic_pin2_to_cpu0() {
        // Linux check_timer()'s primary path: the 8259 masks IRQ0 and the
        // timer is routed via IOAPIC pin 2 (GSI2, the IRQ0->GSI2 override) to
        // CPU 0's LAPIC. A PIT tick must deliver the redirection vector.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();

                // PIT counter 0, mode 2, divisor 100 (~84 us period).
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_CONTROL, 0x34, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 100, 1, 0);
                emu.device_manager
                    .pit
                    .write(crate::iodev::pit::PIT_COUNTER0, 0, 1, 0);
                let now = emu.pc_system.time_ticks();
                let delay = emu.device_manager.pit.next_event_usec();
                emu.devices
                    .request_timer_after_usec(DeviceTimerOwner::Pit, now, delay);
                emu.drain_device_timer_requests();

                // Software-enable CPU 0's LAPIC (spurious vector 0xFF, bit 8).
                emu.cpu_mut().lapic.write_aligned(0xF0, 0x1FF, now);

                // Program IOAPIC pin 2: vector 0x30, fixed, physical dest CPU 0,
                // edge, unmasked. Redirection low index = 0x10 + 2*2 = 0x14.
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0000, 0x14, None, None);
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0010, 0x30, None, None);
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0000, 0x15, None, None);
                emu.device_manager
                    .ioapic
                    .write_aligned(0xFEC0_0010, 0x00, None, None);

                // Linux masks IRQ0 in the 8259 when routing via the IOAPIC.
                emu.device_manager.pic.master.imr |= 0x01;

                // Fire the PIT across several periods.
                emu.service_scheduler_boundary(2_000).unwrap();

                assert_ne!(
                    emu.cpu.pending_event
                        & BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_LAPIC_INTR,
                    0,
                    "IOAPIC pin-2 timer interrupt never raised LAPIC INTR on CPU 0"
                );
                assert_eq!(
                    emu.cpu.lapic.acknowledge_int(),
                    0x30,
                    "CPU 0 LAPIC did not receive the IOAPIC pin-2 timer vector"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn keyboard_one_usec_deadline_ends_up_batch_and_raises_irq1() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(0x1000, &[0x90; 64]).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();
                emu.device_manager.keyboard.send_scancode(0x1E);
                emu.devices.request_timer_after_usec(
                    DeviceTimerOwner::Keyboard,
                    0,
                    Some(1),
                );
                emu.drain_device_timer_requests();

                // The tick-1 fire ends the batch and transfers the byte to the
                // output buffer, but Bochs keyboard.cc periodic() only LATCHES
                // the IRQ on a transfer — it is not raised until the next fire.
                let executed = unsafe { emu.run_cpu_batch(4_096) }.unwrap();
                assert_eq!(executed, 1);
                assert!(emu.device_manager.keyboard.kbd_controller.outb);
                assert_eq!(emu.device_manager.pic.master.irq_in[1], 0);

                // The following serial-delay fire's top-of-function collection
                // raises IRQ1 (one period after the transfer, exactly as Bochs).
                let now = emu.pc_system.time_ticks();
                emu.devices.request_timer_after_usec(
                    DeviceTimerOwner::Keyboard,
                    now,
                    Some(1),
                );
                emu.drain_device_timer_requests();
                unsafe { emu.run_cpu_batch(4_096) }.unwrap();
                assert_ne!(emu.device_manager.pic.master.irq_in[1], 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn deferred_continuous_timer_keeps_its_programmed_period() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const PROGRAMMING_TICKS: u64 = 100;
                const PERIOD_TICKS: u64 = 10;
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::CmosPeriodic,
                        1,
                        false,
                        false,
                        "deferred continuous",
                    )
                    .unwrap();
                emu.device_manager.cmos.periodic_timer_handle = Some(handle);

                emu.devices.request_timer_after_usec_with_mode(
                    DeviceTimerOwner::CmosPeriodic,
                    PROGRAMMING_TICKS,
                    Some(PERIOD_TICKS),
                    true,
                );
                emu.drain_device_timer_requests();

                assert_eq!(
                    emu.pc_system.next_timer_deadline_ticks(),
                    Some(PROGRAMMING_TICKS + PERIOD_TICKS)
                );
                emu.service_scheduler_boundary(PROGRAMMING_TICKS + PERIOD_TICKS)
                    .unwrap();
                assert_eq!(
                    emu.pc_system.next_timer_deadline_ticks(),
                    Some(PROGRAMMING_TICKS + 2 * PERIOD_TICKS),
                    "repeat interval must exclude ticks elapsed before programming"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn deadline_scheduler_preserves_windows_timer_order() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();
                emu.device_manager.keyboard.send_scancode(0x1E);
                emu.devices.request_timer(
                    DeviceTimerOwner::Keyboard,
                    TimerRequest::Activate {
                        deadline_ticks: 1,
                        period_ticks: 1,
                        continuous: false,
                    },
                );
                emu.devices.request_timer(
                    DeviceTimerOwner::CmosOneSecond,
                    TimerRequest::Activate {
                        deadline_ticks: 2,
                        period_ticks: 2,
                        continuous: false,
                    },
                );
                emu.drain_device_timer_requests();
                let uip_handle = emu.device_manager.cmos.uip_timer_handle.unwrap();

                emu.service_scheduler_boundary(1).unwrap();
                assert!(emu.device_manager.keyboard.kbd_controller.outb);
                assert!(!emu.pc_system.is_timer_active(uip_handle));

                emu.service_scheduler_boundary(1).unwrap();
                assert!(emu.pc_system.is_timer_active(uip_handle));
                assert_eq!(emu.pc_system.next_timer_deadline_ticks(), Some(246));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn no_fixed_device_polling_state_remains() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE, &[0x90; 512]).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.pc_system.initialize(1_000_000);
                emu.devices.set_timer_ips(1_000_000);
                emu.register_timer_owners().unwrap();
                let keyboard_handle = emu.device_manager.keyboard.timer_handle().unwrap();
                let one_second_handle =
                    emu.device_manager.cmos.one_second_timer_handle.unwrap();
                let uip_handle = emu.device_manager.cmos.uip_timer_handle.unwrap();

                // Keyboard, CMOS one-second, and CMOS UIP owners share the
                // first exact deadline. Registration order must deliver the
                // one-second owner before UIP, then rearm UIP for its distinct
                // later deadline. Running this through the CPU loop makes
                // duplicate fixed polling observable as a reordered or
                // replaced UIP arm.
                emu.device_manager.keyboard.send_scancode(0x1E);
                for owner in [
                    DeviceTimerOwner::Keyboard,
                    DeviceTimerOwner::CmosOneSecond,
                    DeviceTimerOwner::CmosUip,
                ] {
                    emu.devices.request_timer(
                        owner,
                        TimerRequest::Activate {
                            deadline_ticks: 1,
                            period_ticks: 1,
                            continuous: false,
                        },
                    );
                }
                emu.drain_device_timer_requests();

                let executed = unsafe { emu.run_cpu_batch(512) }.unwrap();
                assert_eq!(executed, 1, "the tied exact deadline must end the batch");
                assert_eq!(emu.pc_system.time_ticks(), 1);
                assert!(emu.device_manager.keyboard.kbd_controller.outb);
                assert!(!emu.pc_system.is_timer_active(keyboard_handle));
                assert!(!emu.pc_system.is_timer_active(one_second_handle));
                assert!(emu.pc_system.is_timer_active(uip_handle));
                assert_eq!(emu.pc_system.timer_countdown(uip_handle), 244);
                let _ = emu.device_manager.cmos.write(
                    0x70,
                    crate::iodev::cmos::REG_STAT_A as u32,
                    1,
                );
                assert_eq!(
                    emu.device_manager.cmos.read(0x71, 1) & 0x80,
                    0,
                    "tied CMOS owners must fire in one-second-then-UIP order"
                );

                let executed = unsafe { emu.run_cpu_batch(512) }.unwrap();
                assert_eq!(executed, 244, "the mixed owner must fire at its exact deadline");
                assert_eq!(emu.pc_system.time_ticks(), 245);
                assert!(!emu.pc_system.is_timer_active(uip_handle));
                assert_eq!(
                    emu.device_manager.keyboard.kbd_controller.outb as u8,
                    1,
                    "the tied keyboard owner must fire exactly once"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn phase1_tests_subpage_guest_blocks_fetch_sequential_instructions() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const BLOCK_SIZE: usize = 1024;
                const START: u64 = (BLOCK_SIZE - 2) as u64;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 1024 * 1024;
                config.host_memory_size = 1024 * 1024;
                config.memory_block_size = BLOCK_SIZE;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // Four NOPs cross the 1 KiB guest-block edge.  The following
                // self-loop ends the trace without executing unrelated zero RAM.
                emu.virt_write(START, &[0x90, 0x90, 0x90, 0x90, 0xeb, 0xfe])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, START);

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(4) }.unwrap();
                assert!(executed >= 5);
                assert!(emu.cpu_ref(BSP_INDEX).icount - before >= 5);
                assert_eq!(emu.reg_read(X86Reg::Rip), START + 4);
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn strict_cpu_batch_limit_does_not_execute_trace_tail() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const START: u64 = 0x1000;
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                emu.virt_write(START, &[0x90, 0x90, 0x90, 0x90, 0xeb, 0xfe])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, START);

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed =
                    unsafe { emu.run_cpu_batch_with_strict_limit(4, true) }.unwrap();

                assert_eq!(executed, 4);
                assert_eq!(emu.cpu_ref(BSP_INDEX).icount - before, 4);
                assert_eq!(emu.reg_read(X86Reg::Rip), START + 4);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn memory_reinitialization_invalidates_every_cpu_host_pin() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                let old_host_base = resident_host_base(&mut emu) as usize;

                for cpu_index in 0..emu.cpu_count() {
                    let entry = &mut emu.cpu_mut_at(cpu_index).dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page_addr = old_host_base as _;
                }
                emu.refresh_tlb_pins();
                for pin in emu.tlb_pins() {
                    assert!(pin.is_range_pinned(old_host_base, old_host_base + MIB));
                }

                emu.init_memory_and_pc_system().unwrap();

                for pin in emu.tlb_pins() {
                    assert!(!pin.is_range_pinned(old_host_base, old_host_base + MIB));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn run_interactive_stops_at_exact_instruction_limit_across_batches() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const START: u64 = 0x1000;
                const LIMIT: u64 = 100_000 + 2;
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                emu.virt_write(START, &[0x90, 0x90, 0x90, 0x90, 0xeb, 0xfa])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, START);

                let executed = emu.run_interactive(LIMIT).unwrap();

                assert_eq!(executed, LIMIT);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn phase1_tests_emulator_loader_respects_sibling_tlb_pins() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = MIB;

                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.memory.set_a20_mask(u64::MAX);

                emu.load_ram(&[0x5a], 0).unwrap();
                let host_base = resident_host_base(&mut emu);
                let entry = &mut emu.cpu_mut_at(AP_INDEX).dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page_addr = host_base as _;
                assert!(!emu.tlb_pins()[AP_INDEX]
                    .is_range_pinned(host_base as usize, host_base as usize + MIB));
                emu.refresh_tlb_pins();
                assert!(emu.tlb_pins()[AP_INDEX]
                    .is_range_pinned(host_base as usize, host_base as usize + MIB));

                assert!(matches!(
                    emu.load_ram(&[0xa5], MIB as u64),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                emu.mem_read(0, &mut retained).unwrap();
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn run_cpu_batch_passes_all_cpu_tlb_pins_to_eviction() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 3 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.memory.set_a20_mask(u64::MAX);
                emu.cpu_mut_at(AP_INDEX).activity_state = CpuActivityState::WaitForSipi;
                emu.service_scheduler_boundary(0).unwrap();

                // Block 0 is pinned only by the non-running AP.  BSP code
                // occupies the second resident slot; its fetch pin protects
                // that slot.  The store targets swapped block 2, so a
                // current-BSP-only pin set would evict the AP's block 0.
                emu.load_ram(&[0x5a], 0).unwrap();
                emu.load_ram(&[0xC6, 0x07, 0xA5, 0xEB, 0xFE], MIB as u64 + 0x1000)
                    .unwrap();
                let host_base = resident_host_base(&mut emu);
                let entry = &mut emu.cpu_mut_at(AP_INDEX).dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page_addr = host_base as _;
                assert!(!emu.tlb_pins()[AP_INDEX]
                    .is_range_pinned(host_base as usize, host_base as usize + MIB));
                emu.refresh_tlb_pins();
                assert!(emu.tlb_pins()[AP_INDEX]
                    .is_range_pinned(host_base as usize, host_base as usize + MIB));


                emu.reg_write(X86Reg::Rip, MIB as u64 + 0x1000);
                emu.reg_write(X86Reg::Rdi, (2 * MIB) as u64);
                let executed = unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert!(executed >= 1);
                assert!(
                    emu.tlb_pins()[AP_INDEX]
                        .is_range_pinned(host_base as usize, host_base as usize + MIB),
                    "a non-running sibling's direct mapping must survive a BSP slice"
                );

                // Both resident slots are pinned: AP owns block 0 and the
                // running BSP owns its code block.  The target remains
                // unavailable; omitting the AP descriptor makes this read
                // succeed after block 0 is incorrectly evicted.
                let mut target = [0];
                assert!(matches!(
                    emu.mem_read((2 * MIB) as u64, &mut target),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                emu.mem_read(0, &mut retained).unwrap();
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn phase1_tests_unsafe_cpu_mutation_keeps_cached_pin_valid() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = MIB;
                config.memory_block_size = MIB;
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.memory.set_a20_mask(u64::MAX);

                emu.load_ram(&[0x5a], 0).unwrap();
                let host_base = resident_host_base(&mut emu);
                let cpu_address = emu.cpu() as *const BxCpuC<Corei7SkylakeX>;
                let entry = &mut unsafe { emu.cpu_mut_unchecked() }.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page_addr = host_base as _;

                assert_eq!(cpu_address, emu.cpu() as *const BxCpuC<Corei7SkylakeX>);
                assert!(!emu.tlb_pins()[BSP_INDEX]
                    .is_range_pinned(host_base as usize, host_base as usize + MIB));
                emu.refresh_tlb_pins();
                assert!(emu.tlb_pins()[BSP_INDEX]
                    .is_range_pinned(host_base as usize, host_base as usize + MIB));
                assert!(matches!(
                    emu.load_ram(&[0xa5], MIB as u64),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                emu.mem_read(0, &mut retained).unwrap();
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn rep_insw32_fast_path_retires_exact_iteration_count() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x2000;
                const COUNT: u64 = 64;
                const UNMAPPED_PORT: u64 = 0x1234;

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // REP INSW with 32-bit address size, then a parking jump.
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, COUNT);
                let timer = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Keyboard,
                        COUNT + 1,
                        false,
                        true,
                        "REP deadline",
                    )
                    .unwrap();
                let ticks_before = emu.pc_system.time_ticks();

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(1) }.unwrap();
                let retired = emu.cpu_ref(BSP_INDEX).icount - before;

                // The trace executes REP INSW plus the following parking jump.
                // The REP contributes exactly COUNT retirements, and the batch
                // reports Bochs icount units (retired instructions, including
                // repeat() iterations) — not handler dispatches.
                assert_eq!(executed, retired);
                assert_eq!(retired, COUNT + 1);
                assert_eq!(emu.pc_system.time_ticks() - ticks_before, retired);
                assert_eq!(
                    emu.pc_system.timer_countdown(timer),
                    0,
                    "the timer due on the final REP retirement must fire in this batch"
                );
                assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DEST_ADDR + COUNT * 2);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn rep_insw32_stops_at_event_budget_across_page_boundary() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x2ff0;
                const EVENT_BUDGET: u64 = 16;
                const COUNT: u64 = 24;
                const IDE_DATA_PORT: u64 = 0x1f0;

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.attach_disk_data(0, 0, vec![0xa5; 512], 1, 1, 1);
                let drive = &mut emu.device_manager.harddrv.channels[0].drives[0];
                drive
                    .controller
                    .status
                    .insert(crate::iodev::harddrv::AtaStatus::DRQ);
                drive.controller.buffer[..512].fill(0xa5);
                drive.controller.buffer_size = 512;
                drive.controller.buffer_index = 0;
                emu.pc_system.initialize(1_000_000);
                emu.cpu.mmio = crate::memory::mmio::MmioRegistry::new();

                // REP INSW crosses from 0x2fff to 0x3000 before the timer
                // deadline. The initialized IDE data buffer makes both chunks
                // use the real bulk-port path.
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, IDE_DATA_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, COUNT);
                emu.pc_system
                    .register_timer(
                        TimerOwner::NullTimer,
                        EVENT_BUDGET,
                        false,
                        true,
                        "rep_insw_deadline",
                    )
                    .unwrap();
                assert_eq!(
                    emu.pc_system.get_num_cpu_ticks_left_next_event(),
                    EVENT_BUDGET as u32
                );

                unsafe { emu.run_cpu_batch(1) }.unwrap();

                assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT - EVENT_BUDGET);
                assert_eq!(
                    emu.reg_read(X86Reg::Rdi),
                    DEST_ADDR + EVENT_BUDGET * 2
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_with_parked_application_processors_reaches_timer_quantum() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = vec![0x90u8; 131_072];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();
                let retired = emu.cpu_ref(BSP_INDEX).icount - before;

                assert!(
                    executed >= 1024,
                    "SMP batch with parked APs collapsed to trace-sized elapsed ticks: {executed}"
                );
                assert!(
                    retired >= 1024,
                    "SMP batch with parked APs collapsed to trace-sized BSP retirement: {retired}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn halted_application_processors_do_not_force_trace_sized_batches() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = vec![0x90u8; 131_072];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                for cpu_index in 1..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.activity_state = CpuActivityState::Hlt;
                    cpu.async_event = 1;
                }

                let before = emu.cpu_ref(BSP_INDEX).icount;
                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();
                let retired = emu.cpu_ref(BSP_INDEX).icount - before;

                assert!(
                    executed >= 1024,
                    "halted APs collapsed elapsed ticks to {executed}"
                );
                assert!(
                    retired >= 1024,
                    "halted APs forced trace-sized BSP retirement: {retired}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn parked_application_processor_ipi_breaks_bsp_batch_for_delivery() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(2, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = [
                    0xC7, 0x05, 0x10, 0x03, 0xE0, 0xFE, 0x00, 0x00, 0x00,
                    0x01, // ICR high: APIC ID 1
                    0xC7, 0x05, 0x00, 0x03, 0xE0, 0xFE, 0x09, 0x06, 0x00,
                    0x00, // ICR low: SIPI vector 0x09
                    0x90, 0x90, 0x90, 0x90,
                ];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let executed = unsafe { emu.run_cpu_batch(4096) }.unwrap();

                assert!(
                    executed < 4096,
                    "parked-AP fast path must return after IPI write, got full batch {executed}"
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active,
                    "SIPI delivery to parked AP was delayed past the BSP batch"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs icache.cc handleSMC: a store that hits a page with cached traces
    /// sets BX_ASYNC_EVENT_STOP_TRACE on the WRITING cpu too, so the remainder
    /// of the currently-executing trace is abandoned and re-decoded from the
    /// (now patched) memory. Without it, the stale tail of the running trace
    /// executes the pre-patch instruction.
    #[test]
    fn smc_store_within_current_trace_stops_trace() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // 0x1000: mov byte [0x100E], 0x41   ; patch "inc eax" -> "inc ecx"
                // 0x1007: nop x7                    ; same trace, past first_bytes
                // 0x100E: inc eax (0x40)            ; stale target
                // 0x100F: hlt
                let mut code = vec![0xC6, 0x05, 0x0E, 0x10, 0x00, 0x00, 0x41];
                code.extend_from_slice(&[0x90; 7]);
                code.push(0x40);
                code.push(0xF4);
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                unsafe { emu.run_cpu_batch(4096) }.unwrap();

                assert_eq!(
                    emu.reg_read(X86Reg::Rcx),
                    1,
                    "patched inc ecx must execute (trace re-decoded after SMC store)"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Rax),
                    0,
                    "stale inc eax executed from the invalidated trace"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The final fw_cfg DMA OUT in this cached trace overwrites the next
    /// decoded `inc ecx` with HLT.  The issuing CPU must abandon its stale
    /// trace tail before it executes that original instruction.
    #[test]
    fn fw_cfg_dma_out_stops_cached_trace_before_following_instruction() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                const DESCRIPTOR: u64 = 0x3000;
                const KEY: u16 = 0x1234;

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // `new_with_mode` skips device initialization, so register the
                // real fw_cfg port handler before running guest OUTs.
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();
                emu.device_manager.fw_cfg.add_bytes(KEY, &[0xF4]);

                // mov edx,0x514; xor eax,eax; out dx,eax;
                // add edx,4; mov eax,bswap(0x3000); out dx,eax;
                // inc ecx; hlt
                let mut code = vec![
                    0xBA, 0x14, 0x05, 0x00, 0x00, 0x31, 0xC0, 0xEF, 0x83, 0xC2, 0x04, 0xB8,
                    0x00, 0x00, 0x30, 0x00, 0xEF,
                ];
                let patched_inc = CODE + code.len() as u64;
                code.extend_from_slice(&[0x41, 0xF4]);

                let control = ((KEY as u32) << 16) | 0x08 | 0x02;
                let mut descriptor = [0u8; 16];
                descriptor[..4].copy_from_slice(&control.to_be_bytes());
                descriptor[4..8].copy_from_slice(&(1u32).to_be_bytes());
                descriptor[8..].copy_from_slice(&patched_inc.to_be_bytes());
                emu.load_ram(&descriptor, DESCRIPTOR).unwrap();
                emu.virt_write(CODE, &code).unwrap();
                emu.reg_write(X86Reg::Rcx, 0);
                emu.reg_write(X86Reg::Rip, CODE);

                unsafe { emu.run_cpu_batch(4096) }.unwrap();

                assert_eq!(emu.mem_read_vec(patched_inc, 1).unwrap(), [0xF4]);
                assert_eq!(emu.cpu_ref(0).activity_state, CpuActivityState::Hlt);
                assert_eq!(
                    emu.reg_read(X86Reg::Rcx),
                    0,
                    "the stale inc ecx after OUT executed before fw_cfg SMC was applied"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs icache.cc handleSMC loops over BX_SMP_PROCESSORS: a write by one
    /// CPU to a page with cached traces must flush EVERY cpu's icache, not just
    /// the writer's. The AP spins on a nop-sled loop whose jmp sits past the
    /// 8-byte first_bytes guard; the BSP patches the jmp to hlt;hlt. Stale
    /// sibling caches spin forever.
    #[test]
    fn smp_cross_cpu_code_patch_invalidates_sibling_icache() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(2, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // AP (real mode, SIPI vector 0x09 -> 0x0900:0000 = phys 0x9000):
                //   0x9000: nop x8
                //   0x9008: jmp 0x9000 (EB F6)
                let mut ap_code = vec![0x90u8; 8];
                ap_code.extend_from_slice(&[0xEB, 0xF6]);
                emu.virt_write(0x9000, &ap_code).unwrap();
                // BSP: spin long enough that the AP has cached its loop trace,
                // then patch the AP's jmp to hlt;hlt, then halt.
                let mut bsp_code = vec![0x90u8; 600];
                // mov word [0x9008], 0xF4F4
                bsp_code.extend_from_slice(&[0x66, 0xC7, 0x05, 0x08, 0x90, 0x00, 0x00, 0xF4, 0xF4]);
                bsp_code.push(0xF4); // hlt
                emu.virt_write(0x1000, &bsp_code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(0x09);
                emu.rebuild_cpu_masks_from_scan();

                for _ in 0..100 {
                    unsafe { emu.run_cpu_batch(4096) }.unwrap();
                    if matches!(emu.cpu_ref(AP_INDEX).activity_state, CpuActivityState::Hlt) {
                        break;
                    }
                }
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Hlt,
                    "AP kept executing a stale cached trace after the BSP patched its code"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs memory.cc dmaWritePhysicalPage -> pageWriteStampTable.decWriteStamp:
    /// a real legacy-DMA physical write must invalidate cached traces exactly
    /// like CPU stores.
    #[test]
    fn dma_write_to_cached_code_page_invalidates_icache() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                fn dma_read(_data: &[u8], _maxlen: u16) -> u16 {
                    0
                }
                fn dma_write(data: &mut [u8], maxlen: u16) -> u16 {
                    let patch = [0xF4, 0xF4];
                    let len = patch.len().min(maxlen as usize);
                    data[..len].copy_from_slice(&patch[..len]);
                    len as u16
                }

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                // 0x1000: nop x8 / 0x1008: jmp 0x1000 — jmp is past first_bytes.
                let mut code = vec![0x90u8; 8];
                code.extend_from_slice(&[0xEB, 0xF6]);
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                // Let the CPU cache and spin the loop trace.
                unsafe { emu.run_cpu_batch(4096) }.unwrap();
                // A real DMA controller write patches the jmp to hlt;hlt.
                let pins_ptr = emu.tlb_pins().as_ptr();
                let pins_len = emu.tlb_pins().len();
                let pins = unsafe { core::slice::from_raw_parts(pins_ptr, pins_len) };
                let dma = &mut emu.device_manager.dma;
                assert!(dma.register_dma8_channel(2, dma_read, dma_write, "SMC test"));
                dma.s[0].mask[2] = false;
                dma.s[1].mask[0] = false;
                dma.s[0].status_reg |= 1 << 6;
                dma.s[1].status_reg |= 1 << 4;
                dma.s[0].chan[2].current_address = 0x1008;
                dma.s[0].chan[2].current_count = 1;
                dma.s[0].chan[2].mode.transfer_type = 1;
                dma.raise_hlda(Some(&mut emu.memory), pins);

                for _ in 0..50 {
                    unsafe { emu.run_cpu_batch(4096) }.unwrap();
                    if matches!(emu.cpu_ref(0).activity_state, CpuActivityState::Hlt) {
                        break;
                    }
                }
                assert_eq!(
                    emu.cpu_ref(0).activity_state,
                    CpuActivityState::Hlt,
                    "CPU kept executing a stale cached trace after DMA overwrote the code page"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn active_cpu_batch_has_no_fixed_millisecond_polling_cap() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = vec![0x90u8; 131_072];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let executed = unsafe { emu.run_cpu_batch(100_000) }.unwrap();
                assert!(
                    executed >= 100_000,
                    "active batch retained a fixed polling cap: {executed}"
                );

                let mut high_ips_config = EmulatorConfig::default();
                high_ips_config.ips = 300_000_000;
                let mut high_ips_emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    high_ips_config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                high_ips_emu.virt_write(0x1000, &code).unwrap();
                high_ips_emu.reg_write(X86Reg::Rip, 0x1000);

                let executed = unsafe { high_ips_emu.run_cpu_batch(100_000) }.unwrap();
                assert!(
                    executed >= 100_000,
                    "configured IPS reintroduced an active polling cap: {executed}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn keyboard_reset_ack_reaches_bios_poll_before_timeout_at_high_configured_ips() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.ips = 300_000_000;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let code = [
                    0xB0, 0xFF, 0xE6, 0x60, 0xB9, 0xFF, 0xFF, 0x00, 0x00, 0xE4, 0x64, 0xA8, 0x01,
                    0x75, 0x0B, 0xE2, 0xF8, 0xC6, 0x05, 0x00, 0x20, 0x00, 0x00, 0xEE, 0xEB, 0x09,
                    0xE4, 0x60, 0xC6, 0x05, 0x00, 0x20, 0x00, 0x00, 0xAA, 0xF4,
                ];
                emu.virt_write(0x1000, &code).unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                for _ in 0..16 {
                    let executed = unsafe { emu.run_cpu_batch(100_000) }.unwrap();
                    if !emu.batch_advanced_pc_system {
                        emu.advance_pc_system_after_cpu_ticks(executed);
                    }

                    if emu.virt_read_u8(0x2000).unwrap() != 0 {
                        break;
                    }
                }

                assert_eq!(
                    emu.virt_read_u8(0x2000).unwrap(),
                    0xaa,
                    "BIOS-style keyboard reset poll timed out before OBF/ACK reached port 0x64"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn status_ips_uses_retired_instructions_not_virtual_wait_ticks() {
        let elapsed = std::time::Duration::from_secs(1);

        assert_eq!(
            status_ips_from_retired_instructions(1_000, 1_081, elapsed),
            81
        );
    }
    #[test]
    fn hlt_wait_step_uses_exact_next_timer_deadline() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();

                assert_eq!(emu.hlt_wait_step_ticks(), u32::MAX);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn hlt_wait_step_respects_near_pc_system_timer() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.pc_system
                    .register_timer(TimerOwner::Lapic(0), 37, false, true, "near_timer")
                    .unwrap();

                assert_eq!(emu.hlt_wait_step_ticks(), 37);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_policy_matches_bochs_quantum() {
        let behind = SlowdownTimerState::decide(500, 600, 0);
        assert_eq!(behind.next_delay_usec, 1_500);
        assert!(!behind.sleep_one_quantum);
        assert_eq!(behind.next_last_time_usec, 1_000);

        let normal = SlowdownTimerState::decide(600, 500, 0);
        assert_eq!(normal.next_delay_usec, 1_000);
        assert!(!normal.sleep_one_quantum);

        let one_second_ahead = SlowdownTimerState::decide(2_000_000, 0, 1_001_000);
        assert_eq!(one_second_ahead.next_delay_usec, 1_000);
        assert!(one_second_ahead.sleep_one_quantum);
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_owner_bounds_hlt_wait() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.sync_slowdown = true;
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.devices.set_timer_ips(u64::from(emu.config.ips));
                emu.register_timer_owners().unwrap();

                let slowdown_ticks = emu
                    .pc_system
                    .usec_to_ticks(SLOWDOWN_QUANTUM_USEC)
                    .unwrap() as u32;
                assert_eq!(emu.hlt_wait_step_ticks(), slowdown_ticks);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_state_reanchors_on_restore() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                use std::io::Cursor;
                let build = || {
                    let config = EmulatorConfig {
                        guest_memory_size: 4 * 1024 * 1024,
                        host_memory_size: 4 * 1024 * 1024,
                        sync_slowdown: true,
                        ..EmulatorConfig::default()
                    };
                    let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                    emu.initialize().unwrap();
                    emu.reset(ResetReason::Hardware).unwrap();
                    emu
                };

                let mut source = build();
                // Dirty the host pacing history the way a long pre-snapshot
                // run would.
                source.slowdown_timer.last_time_usec = 777_000;
                source.service_scheduler_boundary(0).unwrap();
                let mut saved = Vec::new();
                source.save_snapshot(&mut saved).unwrap();

                let mut restored = build();
                restored.slowdown_timer.last_time_usec = 55; // stale target state
                restored
                    .restore_snapshot(&mut Cursor::new(&saved))
                    .unwrap();

                let handle = restored.slowdown_timer.timer_handle.unwrap();
                restored
                    .pc_system
                    .validate_timer_handle_owner(handle, TimerOwner::Slowdown)
                    .unwrap();
                // Host anchors restart: no pre-restore lead/lag survives and
                // the emulated baseline is the restored virtual clock.
                assert_eq!(restored.slowdown_timer.last_time_usec, 0);
                assert_eq!(
                    restored.slowdown_timer.start_emulated_time_usec,
                    restored.pc_system.time_usec()
                );
                // Pacing continues: the one-shot is armed or already queued.
                assert!(
                    restored.pc_system.is_timer_active(handle)
                        || restored.pc_system.has_fired_owner(TimerOwner::Slowdown)
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn slowdown_config_mismatch_rejects_restore() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                use std::io::Cursor;
                let build = |sync_slowdown: bool| {
                    let config = EmulatorConfig {
                        guest_memory_size: 4 * 1024 * 1024,
                        host_memory_size: 4 * 1024 * 1024,
                        sync_slowdown,
                        ..EmulatorConfig::default()
                    };
                    let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                    emu.initialize().unwrap();
                    emu.reset(ResetReason::Hardware).unwrap();
                    emu
                };

                for (save_slowdown, restore_slowdown) in [(true, false), (false, true)] {
                    let mut source = build(save_slowdown);
                    source.service_scheduler_boundary(0).unwrap();
                    let mut saved = Vec::new();
                    source.save_snapshot(&mut saved).unwrap();

                    let mut target = build(restore_slowdown);
                    let error = target
                        .restore_snapshot(&mut Cursor::new(&saved))
                        .unwrap_err();
                    assert_eq!(
                        error.kind(),
                        std::io::ErrorKind::InvalidData,
                        "slowdown cross-config restore (save={save_slowdown}, \
                         restore={restore_slowdown}) must be rejected: {error}"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn hardware_reset_rearms_exact_device_owners() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu =
                    Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.devices.set_timer_ips(u64::from(emu.config.ips));
                emu.register_timer_owners().unwrap();
                let keyboard_handle = emu.device_manager.keyboard.timer_handle().unwrap();
                let one_second_handle =
                    emu.device_manager.cmos.one_second_timer_handle.unwrap();
                emu.pc_system
                    .activate_timer_usec(keyboard_handle, 7, false)
                    .unwrap();
                assert!(emu.pc_system.is_timer_active(keyboard_handle));

                emu.reset(ResetReason::Hardware).unwrap();

                assert_eq!(
                    emu.device_manager.keyboard.timer_handle(),
                    Some(keyboard_handle)
                );
                // Bochs keyboard.cc registers the 8042 serial-delay timer
                // continuous and always active — reset restarts it at the
                // serial_delay period instead of deactivating it.
                assert!(emu.pc_system.is_timer_active(keyboard_handle));
                assert!(emu.pc_system.is_timer_active(one_second_handle));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn software_reset_requests_preserve_device_state() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.device_manager.pic.master.imr = 0x00;

                // Port 92 bit 0 requests software reset while bit 1 requests
                // A20 disable. Reset must discard the queued disable.
                assert!(emu.write_port_92h(0x01));
                assert!(emu.pc_system.get_enable_a20());
                assert!(!emu.device_manager.port92.a20_change_pending);
                assert!(!emu.device_manager.keyboard.a20_change_pending);

                // The keyboard output-port form has the same reset-dominates
                // rule, while the PIC remains untouched by software reset.
                emu.device_manager
                    .keyboard
                    .write(crate::iodev::keyboard::KBD_COMMAND_PORT, 0xD1, 1);
                emu.device_manager
                    .keyboard
                    .write(crate::iodev::keyboard::KBD_DATA_PORT, 0x00, 1);
                emu.service_scheduler_boundary(0).unwrap();

                assert_eq!(
                    emu.device_manager.pic.master.imr, 0x00,
                    "Bochs software reset must not reset devices"
                );
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu.device_manager.port92.a20_gate);
                assert!(emu.device_manager.keyboard.a20_enabled);
                assert!(!emu.device_manager.port92.a20_change_pending);
                assert!(!emu.device_manager.keyboard.a20_change_pending);
                assert!(emu.device_manager.port92.reset_request.is_none());
                assert!(emu.device_manager.keyboard.reset_requested.is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pci_cf9_hardware_reset_resets_device_state() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.device_manager.pic.master.imr = 0x00;
                emu.device_manager.pci2isa.reset_request = Some(ResetReason::Hardware);

                assert!(emu.check_and_handle_resets().unwrap());

                assert_eq!(
                    emu.device_manager.pic.master.imr, 0xFF,
                    "Bochs hardware reset resets devices"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pam_boundary_flushes_all_cpu_mappings() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                let host_base = resident_host_base(&mut emu) as usize;

                for cpu_index in 0..emu.cpu_count() {
                    let entry = &mut emu.cpu_mut_at(cpu_index).dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page_addr = host_base as _;
                }
                emu.refresh_tlb_pins();
                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| pin.is_range_pinned(host_base, host_base + MIB)));

                emu.device_manager.pci_conf_addr = 0x8000_0058;
                emu.device_manager.pci_write(0x0CFD, 0x30, 1);
                assert!(emu.device_manager.pam_needs_update);
                emu.service_scheduler_boundary(0).unwrap();

                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| !pin.is_range_pinned(host_base, host_base + MIB)));
                assert!(emu.memory.memory_type(12, 1));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn a20_port92_and_keyboard_transitions_flush_all_cpu_mappings() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const MIB: usize = 1024 * 1024;
                let mut config = EmulatorConfig::default();
                config.guest_memory_size = 2 * MIB;
                config.host_memory_size = 2 * MIB;
                config.memory_block_size = MIB;
                config.cpu_params = BxParams::default().with_topology(2, 1, 1).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                let host_base = resident_host_base(&mut emu) as usize;

                for cpu_index in 0..emu.cpu_count() {
                    let entry = &mut emu.cpu_mut_at(cpu_index).dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page_addr = host_base as _;
                }
                emu.refresh_tlb_pins();
                emu.write_port_92h(0x00);
                assert!(!emu.pc_system.get_enable_a20());
                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| !pin.is_range_pinned(host_base, host_base + MIB)));

                for cpu_index in 0..emu.cpu_count() {
                    let entry = &mut emu.cpu_mut_at(cpu_index).dtlb.entries[0];
                    entry.lpf = 0;
                    entry.host_page_addr = host_base as _;
                }
                emu.refresh_tlb_pins();
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_COMMAND_PORT,
                    0xDF,
                    1,
                );
                assert!(emu.device_manager.keyboard.a20_change_pending);
                emu.service_scheduler_boundary(0).unwrap();
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu
                    .tlb_pins()
                    .iter()
                    .all(|pin| !pin.is_range_pinned(host_base, host_base + MIB)));

                // Regression for independent controller mirrors. Before the
                // boundary synchronization, each second write matched its own
                // stale mirror and was dropped instead of reaching the global
                // A20 gate.
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_COMMAND_PORT,
                    0xDD,
                    1,
                );
                emu.service_scheduler_boundary(0).unwrap();
                assert!(!emu.pc_system.get_enable_a20());
                emu.device_manager.port92.write(0x02);
                assert!(emu.device_manager.port92.a20_change_pending);
                emu.service_scheduler_boundary(0).unwrap();
                assert!(emu.pc_system.get_enable_a20());

                emu.device_manager.port92.write(0x00);
                assert!(emu.device_manager.port92.a20_change_pending);
                emu.service_scheduler_boundary(0).unwrap();
                assert!(!emu.pc_system.get_enable_a20());
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_COMMAND_PORT,
                    0xD1,
                    1,
                );
                emu.device_manager.keyboard.write(
                    crate::iodev::keyboard::KBD_DATA_PORT,
                    0x03,
                    1,
                );
                emu.service_scheduler_boundary(0).unwrap();
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu.device_manager.port92.a20_gate);
                assert!(emu.device_manager.keyboard.a20_enabled);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn all_reset_ports_stop_before_the_next_instruction() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                let reset_guest = |code: &[u8]| {
                    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                    // `new_with_mode` deliberately omits device initialization.
                    emu.devices.init(&mut emu.memory).unwrap();
                    emu.device_manager
                        .init(&mut emu.devices, &mut emu.memory)
                        .unwrap();
                    emu.virt_write(CODE, code).unwrap();
                    emu.reg_write(X86Reg::Rip, CODE);
                    let executed = unsafe { emu.run_cpu_batch(64) }.unwrap();
                    assert!(executed > 0);
                    assert!(
                        emu.devices.take_port_e9_output().is_empty(),
                        "the visible marker OUT after the reset request executed"
                    );
                    emu
                };

                // mov edx,0x92; mov al,0x01; out dx,al; out 0xe9,al; hlt
                let emu = reset_guest(&[
                    0xBA, 0x92, 0x00, 0x00, 0x00, 0xB0, 0x01, 0xEE, 0xE6, 0xE9, 0xF4,
                ]);
                assert!(emu.pc_system.get_enable_a20());
                assert!(emu.device_manager.port92.a20_gate);
                assert!(emu.device_manager.keyboard.a20_enabled);
                assert!(emu.device_manager.port92.reset_request.is_none());

                // mov edx,0x64; mov al,0xfe; out dx,al; out 0xe9,al; hlt
                let emu = reset_guest(&[
                    0xBA, 0x64, 0x00, 0x00, 0x00, 0xB0, 0xFE, 0xEE, 0xE6, 0xE9, 0xF4,
                ]);
                assert!(emu.device_manager.keyboard.reset_requested.is_none());

                // mov edx,0xcf9; mov al,0x02; out dx,al; mov al,0x06;
                // out dx,al; out 0xe9,al; hlt
                let emu = reset_guest(&[
                    0xBA, 0xF9, 0x0C, 0x00, 0x00, 0xB0, 0x02, 0xEE, 0xB0, 0x06, 0xEE,
                    0xE6, 0xE9, 0xF4,
                ]);
                assert!(emu.device_manager.pci2isa.reset_request.is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn reset_boundary_discards_elapsed_ticks_and_pending_timers() {
        // Bochs pc_system.cc bx_pc_system_c::Reset runs synchronously inside
        // the triggering OUT: no pre-reset elapsed tick, deferred timer
        // request, or queued callback may reach the post-reset machine.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                for hardware in [false, true] {
                    let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                        EmulatorConfig::default(),
                        CpuSetupMode::FlatProtected32,
                    )
                    .unwrap();
                    emu.devices.init(&mut emu.memory).unwrap();
                    emu.device_manager
                        .init(&mut emu.devices, &mut emu.memory)
                        .unwrap();

                    // A device files a pre-reset one-shot request, exactly as
                    // a PIT port write would.
                    let now = emu.pc_system.time_ticks();
                    emu.devices
                        .request_timer_after_usec(DeviceTimerOwner::Pit, now, Some(1));

                    if hardware {
                        emu.device_manager.pci2isa.reset_request =
                            Some(ResetReason::Hardware);
                    } else {
                        emu.device_manager.port92.write(0x01);
                    }

                    let t0 = emu.pc_system.time_ticks();
                    let reset_applied = emu.service_scheduler_boundary(10_000).unwrap();
                    assert!(reset_applied, "reset must be reported by the boundary");

                    // Virtual time did not advance: the elapsed ticks were
                    // discarded, so no timer can fire before the first
                    // instruction at the reset vector.
                    assert_eq!(emu.pc_system.time_ticks(), t0);
                    assert!(!emu.pc_system.has_fired_timers());

                    // The pre-reset PIT request is gone (the post-reset rearm
                    // drained its own requests inside reset()).
                    let table = emu.devices.take_timer_requests();
                    assert_eq!(
                        table.get(DeviceTimerOwner::Pit),
                        TimerRequest::Unchanged,
                        "pre-reset timer request survived the reset boundary"
                    );

                    // CPU is at the reset vector.
                    assert_eq!(emu.reg_read(X86Reg::Rip), 0xFFF0);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn legacy_dma_drq_to_hlda_terminal_count_end_to_end() {
        // Bochs dma.cc: set_DRQ -> control_HRQ -> bx_pc_system.set_HRQ(1) ->
        // CPU handleAsyncEvent -> raise_HLDA -> transfer -> terminal count ->
        // set_HRQ(0). The full chain must work through the deferred-request
        // transport: a DRQ must wake the CPU and the TC must drop HRQ.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                fn dma_write(data: &mut [u8], maxlen: u16) -> u16 {
                    let payload = [0xd1, 0xa5, 0x5e, 0x33];
                    let len = payload.len().min(maxlen as usize);
                    data[..len].copy_from_slice(&payload[..len]);
                    len as u16
                }
                fn dma_read(_data: &[u8], _maxlen: u16) -> u16 {
                    0
                }

                const CODE: u64 = 0x1000;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();

                {
                    let dma = &mut emu.device_manager.dma;
                    assert!(dma.register_dma8_channel(
                        2,
                        dma_read,
                        dma_write,
                        "hrq end-to-end"
                    ));
                    dma.s[0].mask[2] = false;
                    dma.s[1].mask[0] = false;
                    dma.s[0].chan[2].mode.mode_type = 1; // single (Bochs dma.cc)
                    dma.s[0].chan[2].mode.transfer_type = 1; // I/O -> memory
                    dma.s[0].chan[2].page_reg = 0x20;
                    dma.s[0].chan[2].base_count = 3;
                    dma.s[0].chan[2].current_count = 3;
                    dma.set_drq(2, true);
                }

                // The boundary transports the request to pc_system and nudges
                // the CPU (Bochs pc_system.cc set_HRQ).
                assert!(!emu.service_scheduler_boundary(0).unwrap());
                assert!(emu.pc_system.get_hrq());
                assert_ne!(emu.cpu.async_event, 0);

                // nop; hlt — handle_async_event services HLDA before the
                // first instruction completes the batch.
                emu.virt_write(CODE, &[0x90, 0xF4]).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                let executed = unsafe { emu.run_cpu_batch(8) }.unwrap();
                assert!(executed > 0);

                // Payload landed at page_reg 0x20 -> physical 0x20_0000.
                let mut received = [0u8; 4];
                assert_eq!(
                    emu.memory
                        .read_ram(&[], 0x20_0000, &mut received)
                        .unwrap(),
                    4
                );
                assert_eq!(received, [0xd1, 0xa5, 0x5e, 0x33]);

                let dma = &emu.device_manager.dma;
                // Terminal count reached: status bit set, non-autoinit
                // channel re-masked (Bochs dma.cc raise_HLDA).
                assert_ne!(dma.s[0].status_reg & (1 << 2), 0, "TC status not set");
                assert!(dma.s[0].mask[2], "non-autoinit channel must re-mask at TC");
                // The synchronous TC deassert reached pc_system.
                assert!(!emu.pc_system.get_hrq(), "HRQ must drop at terminal count");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn mid_batch_reset_commits_no_pre_reset_ticks() {
        // A guest-triggered reset mid-batch must not commit the instructions
        // executed before the reset as post-reset virtual time.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.devices.init(&mut emu.memory).unwrap();
                emu.device_manager
                    .init(&mut emu.devices, &mut emu.memory)
                    .unwrap();

                // mov edx,0x92; mov al,0x01; out dx,al; out 0xe9,al; hlt
                emu.virt_write(
                    CODE,
                    &[0xBA, 0x92, 0x00, 0x00, 0x00, 0xB0, 0x01, 0xEE, 0xE6, 0xE9, 0xF4],
                )
                .unwrap();
                emu.reg_write(X86Reg::Rip, CODE);

                let t0 = emu.pc_system.time_ticks();
                let executed = unsafe { emu.run_cpu_batch(64) }.unwrap();
                assert!(executed > 0);
                assert!(
                    emu.devices.take_port_e9_output().is_empty(),
                    "the marker OUT after the reset request executed"
                );
                // The three pre-reset instructions were discarded, not
                // committed: the post-reset clock still reads the pre-batch
                // epoch.
                assert_eq!(
                    emu.pc_system.time_ticks(),
                    t0,
                    "pre-reset elapsed ticks were committed to the fresh machine"
                );
                assert_eq!(emu.reg_read(X86Reg::Rip), 0xFFF0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn initialize_enables_configured_pci_vga() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                for (pci_enabled, pci_vga) in
                    [(false, false), (false, true), (true, false), (true, true)]
                {
                    let mut config = EmulatorConfig::default();
                    config.pci_enabled = pci_enabled;
                    config.pci_vga = pci_vga;
                    let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                    emu.initialize().unwrap();

                    let expected = pci_enabled && pci_vga;
                    assert_eq!(emu.device_manager.vga.pci_enabled(), expected);
                    assert_eq!(
                        emu.device_manager.vga.pci_read(0x04, 1),
                        if expected { 0x03 } else { 0xFFFF_FFFF }
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn emulator_new_builds_and_resets_all_configured_cpus() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();

                assert_eq!(emu.cpu_count(), TEST_SMP_PACKAGES as usize);
                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.get_id(), BSP_INDEX as u32);
                assert_eq!(emu.cpu_ref(AP_INDEX).lapic.get_id(), AP_INDEX as u32);

                emu.reset(ResetReason::Hardware).unwrap();

                assert_eq!(
                    emu.cpu_ref(BSP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn nonflat_topology_reports_bochs_compatible_guest_tables() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default().with_topology(2, 2, 2).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                let instr = Instruction::default();

                assert_eq!(emu.cpu_count(), NONFLAT_TOPOLOGY_CPUS as usize);
                for cpu_index in 0..NONFLAT_TOPOLOGY_CPUS as usize {
                    assert_eq!(emu.cpu_ref(cpu_index).lapic.get_id(), cpu_index as u32);

                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.set_eax(CPUID_LEAF_FEATURE_INFO);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(
                        (cpu.ebx() >> CPUID_LEAF1_LOGICAL_COUNT_SHIFT) & CPUID_APIC_ID_BYTE_MASK,
                        4
                    );
                    assert_eq!(
                        (cpu.ebx() >> CPUID_LEAF1_APIC_ID_SHIFT) & CPUID_APIC_ID_BYTE_MASK,
                        cpu_index as u32
                    );

                    cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
                    cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_SMT);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(cpu.eax(), 1);
                    assert_eq!(cpu.ebx(), 2);
                    assert_eq!(
                        cpu.ecx(),
                        topology_level_ecx(
                            CPUID_TOPOLOGY_SUBLEAF_SMT,
                            CPUID_TOPOLOGY_LEVEL_TYPE_SMT
                        )
                    );
                    assert_eq!(cpu.edx(), cpu_index as u32);

                    cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
                    cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_CORE);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(cpu.eax(), BxCpuC::<Corei7SkylakeX>::bochs_topology_shift(4));
                    assert_eq!(cpu.ebx(), 4);
                    assert_eq!(
                        cpu.ecx(),
                        topology_level_ecx(
                            CPUID_TOPOLOGY_SUBLEAF_CORE,
                            CPUID_TOPOLOGY_LEVEL_TYPE_CORE
                        )
                    );
                    assert_eq!(cpu.edx(), cpu_index as u32);

                    cpu.set_eax(CPUID_LEAF_EXTENDED_TOPOLOGY);
                    cpu.set_ecx(CPUID_TOPOLOGY_SUBLEAF_PACKAGE);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!((cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()), (0, 0, 0, 0));
                }

                emu.initialize().unwrap();

                assert_eq!(emu.device_manager.ioapic.apic_id(), NONFLAT_TOPOLOGY_CPUS);
                assert_eq!(
                    read_fw_cfg_u16(&mut emu.device_manager.fw_cfg, FW_CFG_NB_CPUS_KEY),
                    NONFLAT_TOPOLOGY_CPUS as u16
                );
                assert_eq!(
                    read_fw_cfg_u16(&mut emu.device_manager.fw_cfg, FW_CFG_MAX_CPUS_KEY),
                    NONFLAT_TOPOLOGY_CPUS as u16
                );

                let acpi = AcpiTableGenerator::generate(
                    emu.config.guest_memory_size as u64,
                    NONFLAT_TOPOLOGY_CPUS,
                );
                let madt = acpi_madt_from_tables(acpi.tables_blob());
                let (lapic_ids, lapic_count, ioapic_id) =
                    parse_madt_ids::<{ NONFLAT_TOPOLOGY_CPUS as usize }>(madt);

                assert_eq!(lapic_count, NONFLAT_TOPOLOGY_CPUS as usize);
                for (expected_id, actual_id) in lapic_ids.iter().copied().enumerate() {
                    assert_eq!(actual_id, expected_id as u8);
                }
                assert_eq!(ioapic_id, Some(NONFLAT_TOPOLOGY_CPUS as u8));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn cpuid_freq_config_reaches_every_cpu_through_cpuid_instruction() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                // `ips` mode: leaf 0x15 must report a crystal of `ips` Hz with
                // a 1/1 ratio and leaf 0x16 the rate in MHz — on the BSP and
                // on every AP (Bochs cpuid.cc get_freq_leaf_15/16).
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default().with_topology(1, 2, 1).unwrap();
                config.ips = 120_000_000;
                config.cpuid_freq = CpuidFreq::Ips;
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                let instr = Instruction::default();

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.set_eax(0x15);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(
                        (cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()),
                        (1, 1, 120_000_000, 0)
                    );

                    cpu.set_eax(0x16);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(
                        (cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()),
                        (120, 120, 100, 0)
                    );

                    // Max standard leaf stays Bochs-exact 0x16 in every mode
                    // (Bochs corei7_skylake-x.cc max_std_leaf).
                    cpu.set_eax(0);
                    cpu.set_ecx(0);
                    cpu.cpuid(&instr).unwrap();
                    assert_eq!(cpu.eax(), 0x16);
                }

                // Default config (CpuidFreq::None): the frequency leaves read
                // as not enumerated so guests PIT-calibrate the true rate.
                let mut emu = Emulator::<Corei7SkylakeX>::new(EmulatorConfig::default()).unwrap();
                let cpu = emu.cpu_mut_at(0);
                cpu.set_eax(0x15);
                cpu.set_ecx(0);
                cpu.cpuid(&instr).unwrap();
                assert_eq!((cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()), (0, 0, 0, 0));
                cpu.set_eax(0x16);
                cpu.set_ecx(0);
                cpu.cpuid(&instr).unwrap();
                assert_eq!((cpu.eax(), cpu.ebx(), cpu.ecx(), cpu.edx()), (0, 0, 0, 0));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn topology_254_generates_non_wrapping_firmware_ids() {
        let params = BxParams::default().with_topology(127, 2, 1).unwrap();
        let cpu_count = params.cpu_count();
        assert_eq!(cpu_count, MAX_SUPPORTED_TEST_CPUS);

        let mut fw_cfg = crate::iodev::fw_cfg::BxFwCfg::new();
        fw_cfg.init(
            EmulatorConfig::default().guest_memory_size as u64,
            cpu_count,
        );
        assert_eq!(
            read_fw_cfg_u16(&mut fw_cfg, FW_CFG_NB_CPUS_KEY),
            MAX_SUPPORTED_TEST_CPUS as u16
        );
        assert_eq!(
            read_fw_cfg_u16(&mut fw_cfg, FW_CFG_MAX_CPUS_KEY),
            MAX_SUPPORTED_TEST_CPUS as u16
        );

        let acpi = AcpiTableGenerator::generate(
            EmulatorConfig::default().guest_memory_size as u64,
            cpu_count,
        );
        let madt = acpi_madt_from_tables(acpi.tables_blob());
        let (lapic_ids, lapic_count, ioapic_id) =
            parse_madt_ids::<{ MAX_SUPPORTED_TEST_CPUS as usize }>(madt);

        assert_eq!(lapic_count, MAX_SUPPORTED_TEST_CPUS as usize);
        assert_eq!(
            lapic_ids[(MAX_SUPPORTED_TEST_CPUS - 1) as usize],
            (MAX_SUPPORTED_TEST_CPUS - 1) as u8
        );
        assert_eq!(ioapic_id, Some(MAX_SUPPORTED_TEST_CPUS as u8));
        assert_eq!(
            madt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            ACPI_CHECKSUM_VALID_SUM
        );
    }

    #[test]
    fn cpu_masks_preserve_indices_32_and_253_across_scan_oracle() {
        let mut runnable = CpuMask::default();
        let mut lapic_work = CpuMask::default();
        runnable.assign(32, true);
        runnable.assign(253, true);
        lapic_work.assign(32, true);
        lapic_work.assign(253, true);

        let mut scanned_runnable = CpuMask::default();
        let mut scanned_lapic_work = CpuMask::default();
        for index in 0..MAX_SUPPORTED_TEST_CPUS as usize {
            scanned_runnable.assign(index, index == 32 || index == 253);
            scanned_lapic_work.assign(index, index == 32 || index == 253);
        }

        assert_eq!(runnable, scanned_runnable);
        assert_eq!(lapic_work, scanned_lapic_work);
        assert_eq!(runnable.count(MAX_SUPPORTED_TEST_CPUS as usize), 2);
        assert_eq!(runnable.next_set(0, MAX_SUPPORTED_TEST_CPUS as usize), Some(32));
        assert_eq!(runnable.next_set(33, MAX_SUPPORTED_TEST_CPUS as usize), Some(253));
        assert_eq!(runnable.next_set(254, MAX_SUPPORTED_TEST_CPUS as usize), None);

        // The shipped HLT fast-forward predicate at maximum topology. A
        // 254-CPU Emulator is not constructible in tests (each BxCpuC is tens
        // of megabytes), so the production associated fn is exercised at the
        // mask level across every word boundary the full machine would hit;
        // the 2-CPU transition matrix covers the live plumbing.
        type TestEmu<'a> = Emulator<'a, Corei7SkylakeX>;
        const MAX: usize = 254;
        assert!(!TestEmu::ap_fast_forward_allowed(runnable, MAX), "bit 32+253");
        assert!(TestEmu::ap_fast_forward_allowed(CpuMask::default(), MAX));
        for ap_bit in [1usize, 32, 63, 64, 191, 192, 253] {
            let mut mask = CpuMask::default();
            mask.assign(ap_bit, true);
            assert!(
                !TestEmu::ap_fast_forward_allowed(mask, MAX),
                "runnable AP bit {ap_bit} must block fast-forward"
            );
        }
        // A runnable BSP alone never blocks AP fast-forward.
        let mut bsp_only = CpuMask::default();
        bsp_only.assign(0, true);
        assert!(TestEmu::ap_fast_forward_allowed(bsp_only, MAX));
        // An AP bit at/beyond the CPU count is outside the topology.
        let mut beyond = CpuMask::default();
        beyond.assign(200, true);
        assert!(TestEmu::ap_fast_forward_allowed(beyond, 100));
    }

    #[test]
    fn cpu_masks_match_scan_oracle_for_scheduler_transition_matrix() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const FIXED_VECTOR: u32 = 0x42;
                const LEVEL_VECTOR: u8 = 0xE0;
                const ICR_SELF: u32 = 1 << 18;
                const ICR_ALL_INCLUDING_SELF: u32 = 2 << 18;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                // Reset leaves the BSP runnable and the AP waiting for SIPI.
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
                emu.assert_cpu_masks_match_scan();
                // WAIT_FOR_SIPI AP: BSP HLT may fast-forward.
                assert!(emu.can_fast_forward_bsp_hlt());

                // HLT and a local wake event update only the affected CPU bit.
                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.activity_state = CpuActivityState::Hlt;
                    bsp.pending_event = 0;
                    bsp.async_event = 0;
                }
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(!emu.runnable_mask.contains(BSP_INDEX));

                emu.apply_lapic_cpu_event(BSP_INDEX, Some(LocalApicCpuEvent::Nmi));
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(BSP_INDEX));

                // A reset clears the local wake and reinstates WAIT_FOR_SIPI.
                emu.reset(ResetReason::Hardware).unwrap();
                emu.assert_cpu_masks_match_scan();
                emu.apply_lapic_cpu_event(
                    AP_INDEX,
                    Some(LocalApicCpuEvent::Sipi(AP_TRAMPOLINE_VECTOR)),
                );
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(AP_INDEX));
                // SIPI'd (Active) AP: fast-forward is forbidden.
                assert!(!emu.can_fast_forward_bsp_hlt());

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.activity_state = CpuActivityState::Hlt;
                    cpu.set_rflags_for_api(0x202);
                    cpu.pending_event = 0;
                    cpu.async_event = 0;
                    cpu.lapic.intr = false;
                    cpu.lapic.intr_pending = false;
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    emu.refresh_cpu_masks(cpu_index);
                }
                emu.assert_cpu_masks_match_scan();
                // Both halted with no pending events: fast-forward allowed.
                assert!(emu.can_fast_forward_bsp_hlt());

                // A physical-destination IPI exercises one remote LAPIC.
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x310, (AP_INDEX as u32) << 24, 0);
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x300, FIXED_VECTOR, 0);
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.drain_lapic_bus();
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(AP_INDEX));

                // Self shorthand keeps routing and membership local to the BSP.
                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.activity_state = CpuActivityState::Hlt;
                    bsp.pending_event = 0;
                    bsp.async_event = 0;
                    bsp.lapic.intr = false;
                    bsp.lapic.intr_pending = false;
                    bsp.lapic
                        .write_aligned(0x300, ICR_SELF | (FIXED_VECTOR + 1), 0);
                }
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.drain_lapic_bus();
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(BSP_INDEX));

                // All-including-self shorthand updates both target bits.
                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.activity_state = CpuActivityState::Hlt;
                    cpu.pending_event = 0;
                    cpu.async_event = 0;
                    cpu.lapic.intr = false;
                    cpu.lapic.intr_pending = false;
                    emu.refresh_cpu_masks(cpu_index);
                }
                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(
                    0x300,
                    ICR_ALL_INCLUDING_SELF | (FIXED_VECTOR + 2),
                    0,
                );
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.drain_lapic_bus();
                emu.assert_cpu_masks_match_scan();
                assert!(emu.runnable_mask.contains(BSP_INDEX));
                assert!(emu.runnable_mask.contains(AP_INDEX));
                // Halted AP holding a delivered IPI wake: fast-forward
                // is forbidden.
                assert!(!emu.can_fast_forward_bsp_hlt());

                // EOI is deferred local LAPIC work until the central boundary.
                {
                    let lapic = &mut emu.cpu_mut_at(BSP_INDEX).lapic;
                    lapic.deliver(LEVEL_VECTOR, 0, crate::cpu::apic::APIC_LEVEL_TRIGGERED);
                    assert_eq!(lapic.acknowledge_int(), LEVEL_VECTOR);
                    lapic.receive_eoi(0);
                    assert_eq!(lapic.pending_eoi_vector, Some(LEVEL_VECTOR));
                }
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.service_lapic_local_events();
                emu.assert_cpu_masks_match_scan();
                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.pending_eoi_vector, None);

                // Timer programming and fire are both represented as LAPIC work.
                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(1, 0);
                }
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.service_lapic_timer_requests();
                emu.assert_cpu_masks_match_scan();
                emu.cpu_mut_at(AP_INDEX).lapic.timer_fired = true;
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                emu.service_lapic_local_events();
                emu.assert_cpu_masks_match_scan();

                // Mixed two-CPU state: halted BSP, active AP, AP-only LAPIC work.
                emu.reset(ResetReason::Hardware).unwrap();
                emu.apply_lapic_cpu_event(
                    AP_INDEX,
                    Some(LocalApicCpuEvent::Sipi(AP_TRAMPOLINE_VECTOR)),
                );
                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.activity_state = CpuActivityState::Hlt;
                    bsp.pending_event = 0;
                    bsp.async_event = 0;
                }
                emu.cpu_mut_at(AP_INDEX).lapic.timer_fired = true;
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.refresh_cpu_masks(AP_INDEX);
                emu.assert_cpu_masks_match_scan();
                assert!(!emu.runnable_mask.contains(BSP_INDEX));
                assert!(emu.runnable_mask.contains(AP_INDEX));
                assert!(!emu.lapic_work_mask.contains(BSP_INDEX));
                assert!(emu.lapic_work_mask.contains(AP_INDEX));

                emu.reset(ResetReason::Hardware).unwrap();
                emu.assert_cpu_masks_match_scan();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_executes_application_processor_after_sipi() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::WaitForSipi;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                emu.rebuild_cpu_masks_from_scan();
                let before = emu.cpu_ref(AP_INDEX).icount;

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();

                assert!(executed > 0);
                assert!(
                    emu.cpu_ref(AP_INDEX).icount > before,
                    "active AP did not receive a CPU batch"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bsp_icr_init_sipi_wakes_and_runs_application_processor() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const ICR_LOW: u64 = 0x300;
                const ICR_HIGH: u64 = 0x310;
                const TARGET_APIC_ID: u32 = 1;
                const ICR_LEVEL_ASSERT: u32 = 1 << 14;
                const ICR_TRIGGER_LEVEL: u32 = 1 << 15;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                let before = emu.cpu_ref(AP_INDEX).icount;

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.write_aligned(ICR_HIGH, TARGET_APIC_ID << 24, 0);
                    bsp.lapic.write_aligned(ICR_LOW, ((crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8)
                        | ICR_LEVEL_ASSERT
                        | ICR_TRIGGER_LEVEL, 0);
                    bsp.lapic.write_aligned(ICR_LOW, (crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8 | ICR_TRIGGER_LEVEL, 0);
                    bsp.lapic.write_aligned(ICR_HIGH, TARGET_APIC_ID << 24, 0);
                    bsp.lapic.write_aligned(ICR_LOW, AP_TRAMPOLINE_VECTOR as u32
                        | ((crate::cpu::apic::ApicDeliveryMode::Sipi as u32) << 8)
                        | ICR_LEVEL_ASSERT, 0);
                }
                emu.refresh_cpu_masks(BSP_INDEX);

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();

                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).get_cs_selector(),
                    (AP_TRAMPOLINE_VECTOR as u16) << 8
                );
                assert!(emu.cpu_ref(AP_INDEX).rip() > 0);
                assert!(emu.cpu_ref(AP_INDEX).icount > before);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bsp_icr_init_sipi_restarts_active_application_processor() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const SECOND_TRAMPOLINE_VECTOR: u8 = AP_TRAMPOLINE_VECTOR + 1;
                const SECOND_TRAMPOLINE_ADDR: u64 = (SECOND_TRAMPOLINE_VECTOR as u64) << 12;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], SECOND_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );

                // Bochs deliver_INIT only signals the event; the AP resets at
                // its next instruction boundary. A SIPI sent in the same drain
                // would be dropped ("was not halted at the time"), exactly as
                // in Bochs — so the INIT must be processed before the SIPI is
                // sent, mirroring the MP-spec INIT/SIPI delay.
                send_bsp_icr_init(&mut emu);
                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();
                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi,
                    "INIT must software-reset the AP at its next boundary"
                );

                send_bsp_icr_sipi(&mut emu, SECOND_TRAMPOLINE_VECTOR);
                let before = emu.cpu_ref(AP_INDEX).icount;

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();

                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).get_cs_selector(),
                    (SECOND_TRAMPOLINE_VECTOR as u16) << 8
                );
                assert!(emu.cpu_ref(AP_INDEX).rip() > 0);
                assert!(emu.cpu_ref(AP_INDEX).icount > before);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn init_does_not_recall_ipis_already_sent_by_active_ap() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const IN_FLIGHT_VECTOR: u32 = 0x44;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.write_aligned(0x310, (BSP_INDEX as u32) << 24, 0);
                    ap.lapic.write_aligned(0x300, IN_FLIGHT_VECTOR, 0);
                }
                emu.refresh_cpu_masks(AP_INDEX);
                assert!(!emu.cpu_ref(BSP_INDEX).lapic.intr);

                send_bsp_icr_init(&mut emu);
                emu.drain_lapic_bus();

                // Bochs apic.cc send_ipi delivers the AP's ICR write to the
                // bus before the INIT is even processed — an INIT does not
                // recall an IPI that is already in flight.
                assert!(
                    emu.cpu_ref(BSP_INDEX).lapic.intr,
                    "the AP's in-flight IPI must reach the BSP despite the INIT"
                );
                // The INIT itself is only signaled; the AP resets at its next
                // instruction boundary (Bochs event.cc handleAsyncEvent).
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active
                );
                assert!(emu
                    .cpu_ref(AP_INDEX)
                    .is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT));

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();
                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn init_ipi_to_active_ap_stays_pending_until_instruction_boundary() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                send_bsp_icr_init(&mut emu);
                emu.drain_lapic_bus();

                // Bochs deliver_INIT: signal only — no reset from the bus.
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::Active,
                    "INIT must not reset the AP before its next boundary"
                );
                assert!(emu
                    .cpu_ref(AP_INDEX)
                    .is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT));

                let executed = unsafe { emu.run_cpu_batch(AP_BATCH_INSTRUCTIONS) }.unwrap();
                assert!(executed > 0);
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).activity_state,
                    CpuActivityState::WaitForSipi
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smi_then_init_ipis_are_both_signaled_not_collapsed() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24, 0);
                    bsp.lapic.write_aligned(ICR_LOW, (crate::cpu::apic::ApicDeliveryMode::Smi as u32) << 8, 0);
                }
                send_bsp_icr_init(&mut emu);
                emu.drain_lapic_bus();

                // Bochs signals both events; SMI is processed before INIT at
                // the AP's next boundary. An eager INIT reset would clear
                // pending_event and destroy the SMI.
                let ap = emu.cpu_ref(AP_INDEX);
                assert!(
                    ap.is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_SMI),
                    "SMI queued before INIT must survive the drain"
                );
                assert!(
                    ap.is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_INIT),
                    "INIT must be pending alongside the SMI"
                );
                assert_eq!(ap.activity_state, CpuActivityState::Active);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_uses_smp_time_when_peer_is_started_but_halted() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;
                emu.cpu_mut().lapic.intr = false;
                emu.cpu_mut().lapic.intr_pending = false;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                emu.rebuild_cpu_masks_from_scan();

                assert!(
                    !emu.cpu_runnable_for_batch(BSP_INDEX),
                    "test requires a started halted peer with no runnable event"
                );
                assert!(emu.cpu_runnable_for_batch(AP_INDEX));

                let before = emu.cpu_ref(AP_INDEX).icount;
                let quantum = emu.smp_quantum_ticks();
                let elapsed = unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                let ap_delta = emu.cpu_ref(AP_INDEX).icount - before;

                assert!(elapsed >= quantum);
                assert!(
                    elapsed < ap_delta,
                    "elapsed ticks {elapsed} were not averaged with the halted peer quantum; AP delta was {ap_delta}"
                );
                assert_eq!(
                    emu.smp_tick_remainder,
                    (quantum + ap_delta) % 2
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_keeps_guest_time_at_the_frozen_round_epoch() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[0x0F, 0x31, 0xF4], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;
                emu.cpu_mut().lapic.intr = false;
                emu.cpu_mut().lapic.intr_pending = false;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                emu.rebuild_cpu_masks_from_scan();

                let quantum = emu.smp_quantum_ticks();
                let _elapsed = unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                assert_eq!(
                    emu.cpu_ref(AP_INDEX).rax(),
                    0,
                    "AP RDTSC observed peer elapsed time before the round boundary"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_services_lapic_timer_at_round_boundary() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(AP_INDEX), 1, false, false, "ap_lapic")
                    .unwrap();

                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;
                emu.cpu_mut().lapic.intr = false;
                emu.cpu_mut().lapic.intr_pending = false;

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(1, 0);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                }
                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();

                let quantum = emu.smp_quantum_ticks();
                let elapsed = unsafe { emu.run_cpu_batch(quantum) }.unwrap();

                assert!(elapsed > 0);
                assert!(
                    emu.pc_system.time_ticks() > 0,
                    "SMP batch did not advance pc_system at the Bochs round boundary"
                );
                assert!(
                    emu.cpu_ref(AP_INDEX).lapic.intr,
                    "AP LAPIC timer did not interrupt during the SMP batch"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// Bochs main.cc SMP loop: `ticksTotal` grows by BX_TICKN once per round,
    /// so apic.cc get_current_timer_count — frozen within a trace — still
    /// advances between rounds. A CPU whose LAPIC has no queued scheduler
    /// work must therefore observe TMCCT moving across SMP rounds; a frozen
    /// TMCCT hangs guest APIC-timer calibration during AP bring-up.
    #[test]
    fn smp_lapic_tmcct_advances_across_rounds_without_scheduler_work() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const HUGE_TMICT: u32 = 0x0FFF_FFFF;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.load_ram(&[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(AP_INDEX), 1, false, false, "ap_lapic")
                    .unwrap();

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(HUGE_TMICT, 0);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                }
                emu.rebuild_cpu_masks_from_scan();
                // Apply the deferred timer activation; afterwards the AP LAPIC
                // has no scheduler work left, exactly like a guest spinning in
                // an APIC-timer calibration loop.
                emu.service_scheduler_boundary(0).unwrap();

                let read_tmcct = |emu: &mut Emulator<'_, Corei7SkylakeX>| {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    let icount = ap.icount;
                    ap.lapic.read_aligned(0x390, icount)
                };

                let quantum = emu.smp_quantum_ticks();
                let first = read_tmcct(&mut emu);
                for _ in 0..8 {
                    unsafe { emu.run_cpu_batch(quantum) }.unwrap();
                }
                let later = read_tmcct(&mut emu);

                assert!(
                    later < first,
                    "TMCCT frozen across SMP rounds ({later:#x} vs {first:#x}): \
                     round epoch was not stamped into the LAPIC time base"
                );
                assert!(later > 0, "huge TMICT must not have expired in this window");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bsp_hlt_fast_forward_stays_available_until_application_processors_start() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                assert!(
                    emu.can_fast_forward_bsp_hlt(),
                    "APs waiting for SIPI must not disable the BSP HLT real-time pacing path"
                );

                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
                // Direct state poke: production SIPI delivery refreshes the
                // masks itself (refresh_cpu_masks contract); mirror it here.
                emu.refresh_cpu_masks(AP_INDEX);

                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "once an AP is active, the SMP scheduler must own HLT progress"
                );
                emu.assert_cpu_masks_match_scan();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn all_but_self_fixed_ipi_wakes_halted_application_processors() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const FIXED_IPI_VECTOR: u32 = 0xF1;
                const ICR_ALL_BUT_SELF: u32 = 3 << 18;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default().with_topology(2, 2, 2).unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                for cpu_index in 0..emu.cpu_count() {
                    let cpu = emu.cpu_mut_at(cpu_index);
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    if cpu_index != BSP_INDEX {
                        cpu.activity_state = CpuActivityState::Hlt;
                        cpu.set_rflags_for_api(0x202);
                        cpu.pending_event = 0;
                        cpu.async_event = 0;
                        cpu.lapic.intr = false;
                        cpu.lapic.intr_pending = false;
                    }
                }

                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(0x300, ICR_ALL_BUT_SELF | FIXED_IPI_VECTOR, 0);
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.drain_lapic_bus();

                for cpu_index in 1..emu.cpu_count() {
                    assert!(
                        emu.cpu_ref(cpu_index).lapic.intr,
                        "AP {cpu_index} did not receive fixed IPI in LAPIC IRR/INTR"
                    );
                    assert!(
                        emu.cpu_ref(cpu_index).pending_event
                            & BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_LAPIC_INTR
                            != 0,
                        "AP {cpu_index} did not get a CPU LAPIC event bit"
                    );
                    assert!(
                        emu.cpu_runnable_for_batch(cpu_index),
                        "halted AP {cpu_index} was not made runnable by fixed IPI"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn nmi_ipi_wakes_shutdown_application_processor() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const ICR_DELIVERY_NMI: u32 = 4 << 8;
                const NMI_HANDLER_SEG: u16 = 0x0500;
                const NMI_HANDLER_ADDR: u64 = (NMI_HANDLER_SEG as u64) << 4;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();

                // Real-mode IVT entry 2 (NMI) -> NMI_HANDLER_SEG:0000, handler = HLT.
                let ivt_entry: [u8; 4] = [
                    0x00,
                    0x00,
                    (NMI_HANDLER_SEG & 0xFF) as u8,
                    (NMI_HANDLER_SEG >> 8) as u8,
                ];
                emu.load_ram(&ivt_entry, 8).unwrap();
                emu.load_ram(&[0xF4], NMI_HANDLER_ADDR).unwrap();

                for cpu_index in 0..emu.cpu_count() {
                    emu.cpu_mut_at(cpu_index).lapic.write_aligned(0xF0, 0x1FF, 0);
                }

                // SIPI-start the AP (unmasks NMI per Bochs deliver_SIPI), then
                // put it into the triple-fault SHUTDOWN state.
                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                    ap.activity_state = CpuActivityState::Shutdown;
                    ap.pending_event = 0;
                    ap.async_event = 0;
                }
                emu.cpu_mut().activity_state = CpuActivityState::Hlt;
                emu.cpu_mut().pending_event = 0;
                emu.cpu_mut().async_event = 0;

                assert!(
                    !emu.cpu_runnable_for_batch(AP_INDEX),
                    "shutdown AP with no pending event must stay unscheduled"
                );
                assert!(
                    emu.can_fast_forward_bsp_hlt(),
                    "idle shutdown AP must not disable the BSP HLT pacing path"
                );

                // BSP sends a physical-destination NMI IPI to the AP.
                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(0x310, (AP_INDEX as u32) << 24, 0);
                emu.cpu_mut_at(BSP_INDEX).lapic.write_aligned(0x300, ICR_DELIVERY_NMI, 0);
                emu.refresh_cpu_masks(BSP_INDEX);
                emu.drain_lapic_bus();

                assert!(
                    emu.cpu_ref(AP_INDEX)
                        .is_unmasked_event_pending(BxCpuC::<Corei7SkylakeX>::BX_EVENT_NMI),
                    "NMI IPI was not signaled on the shutdown AP"
                );
                assert!(
                    emu.cpu_runnable_for_batch(AP_INDEX),
                    "pending NMI must make a shutdown AP schedulable (Bochs \
                     event.cc handleWaitForEvent wakes SHUTDOWN like HLT)"
                );
                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "pending NMI on a shutdown AP must disable BSP HLT fast-forward"
                );

                let baseline_icount = emu.cpu_ref(AP_INDEX).icount;
                unsafe { emu.run_cpu_batch(256) }.unwrap();

                let ap = emu.cpu_ref(AP_INDEX);
                assert!(
                    !matches!(ap.activity_state, CpuActivityState::Shutdown),
                    "AP did not leave SHUTDOWN after NMI"
                );
                assert!(
                    ap.icount > baseline_icount,
                    "AP did not execute the NMI handler"
                );
                assert_eq!(
                    ap.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                        .selector
                        .value,
                    NMI_HANDLER_SEG,
                    "AP did not vector through IVT entry 2 to the NMI handler"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }


    #[test]
    fn lapic_timer_request_is_activated_before_round_ticks_advance() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(BSP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "bsp_lapic",
                    )
                    .unwrap();
                let programmed_at = emu.pc_system.time_ticks();
                {
                    let cpu = emu.cpu_mut_at(BSP_INDEX);
                    cpu.lapic.timer_handle = Some(handle);
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    cpu.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    cpu.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                    assert!(cpu.lapic.timer_activate_request.is_some());
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_timer_requests();

                assert!(emu
                    .cpu_ref(BSP_INDEX)
                    .lapic
                    .timer_activate_request
                    .is_none());
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    programmed_at + TEST_LAPIC_TIMER_PERIOD_TICKS
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn self_ipi_control_event_ends_up_batch() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                const SELF_NMI_IPI: u32 =
                    (crate::cpu::apic::ApicDeliveryMode::Nmi as u32) << 8;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0xF0, 0x1FF, 0);
                // mov dword ptr [FEE00300], SELF_NMI_IPI; inc ebx; hlt
                emu.virt_write(
                    CODE,
                    &[
                        0xC7,
                        0x05,
                        0x00,
                        0x03,
                        0xE0,
                        0xFE,
                        SELF_NMI_IPI as u8,
                        (SELF_NMI_IPI >> 8) as u8,
                        (SELF_NMI_IPI >> 16) as u8,
                        (SELF_NMI_IPI >> 24) as u8,
                        0xFF,
                        0xC3,
                        0xF4,
                    ],
                )
                .unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rbx, 0);

                unsafe { emu.run_cpu_batch(64) }.unwrap();

                assert_eq!(
                    emu.reg_read(X86Reg::Rbx),
                    0,
                    "sentinel after the self-targeted NMI IPI executed before the boundary"
                );
                assert_ne!(
                    emu.cpu_ref(BSP_INDEX).pending_event
                        & BxCpuC::<Corei7SkylakeX>::BX_EVENT_NMI,
                    0,
                    "the queued self-targeted NMI was not committed at the boundary"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn up_lapic_timer_uses_programming_instruction_epoch() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE: u64 = 0x1000;
                const INITIAL_COUNT: u32 = 4;
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(BSP_INDEX), 1, false, false, "lapic")
                    .unwrap();
                {
                    let cpu = emu.cpu_mut_at(BSP_INDEX);
                    cpu.lapic.timer_handle = Some(handle);
                    cpu.lapic.write_aligned(0xF0, 0x1FF, 0);
                    cpu.lapic
                        .write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                }
                // mov dword ptr [FEE00380], INITIAL_COUNT; inc ebx; hlt
                emu.virt_write(
                    CODE,
                    &[
                        0xC7,
                        0x05,
                        0x80,
                        0x03,
                        0xE0,
                        0xFE,
                        INITIAL_COUNT as u8,
                        (INITIAL_COUNT >> 8) as u8,
                        (INITIAL_COUNT >> 16) as u8,
                        (INITIAL_COUNT >> 24) as u8,
                        0xFF,
                        0xC3,
                        0xF4,
                    ],
                )
                .unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rbx, 0);

                unsafe { emu.run_cpu_batch(64) }.unwrap();

                let timer_period = emu
                    .cpu_ref(BSP_INDEX)
                    .lapic
                    .timer_period_ticks()
                    .expect("guest initial count must arm the LAPIC timer");
                assert_eq!(
                    emu.reg_read(X86Reg::Rbx),
                    0,
                    "sentinel after timer programming executed before the boundary"
                );
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    timer_period,
                    "LAPIC deadline must be based on the programming instruction epoch"
                );
                assert_eq!(
                    emu.pc_system.time_ticks(),
                    1,
                    "only the programming instruction may retire before the boundary"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn halted_application_processor_lapic_timer_disables_bsp_hlt_fast_forward() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.cpu_mut().activity_state = CpuActivityState::Hlt;

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.activity_state = CpuActivityState::Hlt;
                    ap.set_rflags_for_api(0x202);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, 0x30, 0);
                    ap.lapic.set_initial_timer_count(1, 0);
                    ap.lapic.timer_fired = true;
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.sync_event_flags();

                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "a halted AP with a pending LAPIC timer interrupt must re-enter SMP scheduling"
                );
                assert!(
                    emu.cpu_runnable_for_batch(AP_INDEX),
                    "AP LAPIC timer interrupt was not surfaced as a runnable CPU event"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn service_lapic_local_events_catches_up_overdue_periodic_ap_timers() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(AP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "ap_lapic",
                    )
                    .unwrap();

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, LVT_TIMER_PERIODIC_MODE | TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();
                emu.service_scheduler_boundary(TEST_LAPIC_TIMER_ELAPSED_TICKS as u64)
                    .unwrap();

                assert_eq!(
                    emu.cpu_ref(AP_INDEX).lapic.diag_timer_fires,
                    TEST_LAPIC_TIMER_ELAPSED_TICKS as u64 / TEST_LAPIC_TIMER_PERIOD_TICKS
                );
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    TEST_LAPIC_TIMER_ELAPSED_TICKS as u64 + TEST_LAPIC_TIMER_PERIOD_TICKS
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn service_lapic_local_events_catches_up_overdue_periodic_ap_timers_beyond_previous_cap() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const ELAPSED_TICKS: u32 = 10_050;
                const EXPECTED_FIRES: u64 = 1005;
                const EXPECTED_NEXT_FIRE: u64 = 10_060;

                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(TEST_SMP_PACKAGES, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(AP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "ap_lapic",
                    )
                    .unwrap();

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF, 0);
                    ap.lapic.write_aligned(0x320, LVT_TIMER_PERIODIC_MODE | TEST_LAPIC_TIMER_VECTOR, 0);
                    ap.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();
                emu.service_scheduler_boundary(ELAPSED_TICKS as u64)
                    .unwrap();

                assert_eq!(emu.cpu_ref(AP_INDEX).lapic.diag_timer_fires, EXPECTED_FIRES);
                assert_eq!(
                    emu.pc_system.timers[handle].time_to_fire,
                    EXPECTED_NEXT_FIRE
                );
                assert!(emu.cpu_ref(AP_INDEX).lapic.intr);
                assert_ne!(
                    emu.cpu_ref(AP_INDEX).pending_event
                        & BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_LAPIC_INTR,
                    0
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn reset_deactivates_lapic_pc_timer_before_next_tick() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let handle = emu
                    .pc_system
                    .register_timer(
                        TimerOwner::Lapic(BSP_INDEX),
                        TEST_LAPIC_TIMER_PERIOD_TICKS,
                        false,
                        false,
                        "bsp_lapic",
                    )
                    .unwrap();

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.timer_handle = Some(handle);
                    bsp.lapic.write_aligned(0xF0, 0x1FF, 0);
                    bsp.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR, 0);
                    bsp.lapic.set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32, 0);
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();
                assert!(emu.pc_system.is_timer_active(handle));

                emu.reset(ResetReason::Hardware).unwrap();
                emu.pc_system
                    .tickn((TEST_LAPIC_TIMER_PERIOD_TICKS as u32) + 1);
                emu.dispatch_timer_fires();

                assert!(!emu.cpu_ref(BSP_INDEX).lapic.timer_fired);
                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.diag_timer_fires, 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn service_lapic_local_events_drains_level_triggered_eoi() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const VECTOR: u8 = 0x40;
                const VECTOR_BIT: u32 = 1;

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();

                {
                    let lapic = &mut emu.cpu_mut_at(BSP_INDEX).lapic;
                    lapic.write_aligned(0xF0, 0x1FF, 0);
                    lapic.deliver(VECTOR, 0, crate::cpu::apic::APIC_LEVEL_TRIGGERED);
                    assert_eq!(lapic.read_aligned(0x220, 0) & VECTOR_BIT, VECTOR_BIT);
                    assert_eq!(lapic.read_aligned(0x1A0, 0) & VECTOR_BIT, VECTOR_BIT);

                    let acknowledged = lapic.acknowledge_int();
                    assert_eq!(acknowledged, VECTOR);
                    assert_eq!(lapic.read_aligned(0x220, 0) & VECTOR_BIT, 0);
                    assert_eq!(lapic.read_aligned(0x120, 0) & VECTOR_BIT, VECTOR_BIT);
                    assert_eq!(lapic.read_aligned(0x1A0, 0) & VECTOR_BIT, VECTOR_BIT);

                    lapic.receive_eoi(0);
                    assert_eq!(lapic.read_aligned(0x120, 0) & VECTOR_BIT, 0);
                    assert_eq!(lapic.read_aligned(0x1A0, 0) & VECTOR_BIT, 0);
                    assert_eq!(lapic.pending_eoi_vector, Some(VECTOR));
                }

                emu.rebuild_cpu_masks_from_scan();
                emu.service_lapic_local_events();

                assert_eq!(emu.cpu_ref(BSP_INDEX).lapic.pending_eoi_vector, None);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn wait_for_sipi_application_processors_credit_quantum_ticks() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.cpu_params = BxParams::default()
                    .with_topology(8, TEST_SMP_CORES, TEST_SMP_THREADS)
                    .unwrap();
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    config,
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(0x1000, &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN])
                    .unwrap();
                emu.reg_write(X86Reg::Rip, 0x1000);

                let cpu_count = emu.cpu_count() as u64;
                let before = emu.cpu_ref(BSP_INDEX).icount;
                // batch_size=1 finishes after the first round: the quantum
                // credits alone guarantee elapsed >= 1.
                let elapsed = unsafe { emu.run_cpu_batch(1) }.unwrap();

                let retired = emu.cpu_ref(BSP_INDEX).icount - before;
                // Bochs main.cc bx_begin_simulation: every CPU that executes
                // nothing — including APs parked in WAIT_FOR_SIPI — is
                // credited one SMP quantum ("if (n == 0) n = quantum"), and
                // time advances by executed / BX_SMP_PROCESSORS.
                let expected = (retired + emu.smp_quantum_ticks() * (cpu_count - 1)) / cpu_count;
                assert_eq!(
                    elapsed, expected,
                    "SMP round must advance (retired + quantum credits) / cpu_count \
                     ticks for {retired} retired BSP instructions"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn direct_boot_madt_uses_configured_cpu_count() {
        let madt = build_direct_boot_madt(DIRECT_BOOT_MADT_TEST_CPUS);
        let mut offset = DIRECT_MADT_HEADER_SIZE;
        let mut lapic_ids = [UNSET_APIC_ID; DIRECT_BOOT_MADT_TEST_CPUS as usize];
        let mut lapic_count = 0usize;
        let mut ioapic_id = None;

        while offset < madt.len() {
            let entry_type = madt[offset + MADT_ENTRY_TYPE_OFFSET];
            let entry_len = madt[offset + MADT_ENTRY_LENGTH_OFFSET] as usize;
            match entry_type {
                DIRECT_MADT_ENTRY_TYPE_LAPIC => {
                    lapic_ids[lapic_count] = madt[offset + MADT_LAPIC_APIC_ID_OFFSET];
                    lapic_count += 1;
                }
                DIRECT_MADT_ENTRY_TYPE_IOAPIC => {
                    ioapic_id = Some(madt[offset + MADT_IOAPIC_ID_OFFSET]);
                }
                _ => {}
            }
            offset += entry_len;
        }

        assert_eq!(lapic_count, DIRECT_BOOT_MADT_TEST_CPUS as usize);
        assert_eq!(lapic_ids, EXPECTED_DIRECT_BOOT_LAPIC_IDS);
        assert_eq!(ioapic_id, Some(DIRECT_BOOT_MADT_TEST_CPUS as u8));
        assert_eq!(
            madt.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            ACPI_CHECKSUM_VALID_SUM
        );
    }

    #[test]
    fn pic_deferred_clear_cannot_erase_reasserted_int_pin() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.cpu
                    .clear_event(BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_INTR);
                emu.device_manager.pic.irq_pending = true;
                emu.device_manager.pic.irq_cleared = true;
                emu.device_manager.pic.master.int_pin = true;

                emu.sync_event_flags();

                assert_ne!(
                    emu.cpu().pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_INTR,
                    0
                );
                assert!(!emu.device_manager.pic.irq_pending);
                assert!(!emu.device_manager.pic.irq_cleared);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn scheduler_boundary_republishes_asserted_pic_without_edge_history() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.device_manager.pic.master.int_pin = true;
                emu.device_manager.pic.irq_pending = false;
                emu.device_manager.pic.irq_cleared = false;
                emu.cpu
                    .clear_event(BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_INTR);

                emu.sync_event_flags();

                assert_ne!(
                    emu.cpu().pending_event & BxCpuC::<Corei7SkylakeX>::BX_EVENT_PENDING_INTR,
                    0
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn rep_insw_respects_configured_page_write_permissions() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x2000;
                const UNMAPPED_PORT: u64 = 0x1234;

                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.mem_write(DEST_ADDR, &[0x34, 0x12]).unwrap();
                emu.mem_protect(
                    DEST_ADDR,
                    0x1000,
                    crate::cpu::instrumentation::MemPerms::READ,
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, 1);

                unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert_eq!(emu.mem_read_vec(DEST_ADDR, 2).unwrap(), [0x34, 0x12]);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn split_page_rmw_faults_before_first_mmio_read() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                use std::sync::{
                    atomic::{AtomicUsize, Ordering},
                    Arc,
                };

                const CODE_ADDR: u64 = 0x10_0000;
                const DEST_ADDR: u64 = 0x1F_FFFF;
                const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;
                const UNMAPPED_PORT: u64 = 0x1234;

                let reads = Arc::new(AtomicUsize::new(0));
                let read_count = Arc::clone(&reads);
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatLong64,
                )
                .unwrap();
                emu.mem_write(CODE_ADDR, &[0x66, 0x6D, 0xEB, 0xFE]).unwrap();
                // The first byte remains mapped at the end of the first 2 MiB
                // page; the second byte faults in the now-nonpresent page.
                emu.mem_write(SECOND_LARGE_PAGE_PDE, &0u64.to_le_bytes())
                    .unwrap();
                emu.mmio_map(
                    DEST_ADDR,
                    1,
                    Box::new(move |_addr, _size| {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        0
                    }),
                    Box::new(|_addr, _size, _value| {}),
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);

                unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert_eq!(emu.reg_read(X86Reg::Cr2), 0x20_0000);
                assert_eq!(
                    reads.load(Ordering::SeqCst),
                    0,
                    "first MMIO byte was consumed before second-page translation"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn insw_permission_fault_precedes_destructive_mmio_read() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                use std::sync::{
                    atomic::{AtomicUsize, Ordering},
                    Arc,
                };

                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x20_0000;
                const UNMAPPED_PORT: u64 = 0x1234;

                let reads = Arc::new(AtomicUsize::new(0));
                let writes = Arc::new(AtomicUsize::new(0));
                let read_count = Arc::clone(&reads);
                let write_count = Arc::clone(&writes);
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE_ADDR, &[0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.mmio_map(
                    DEST_ADDR,
                    2,
                    Box::new(move |_addr, _size| {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        0
                    }),
                    Box::new(move |_addr, _size, _value| {
                        write_count.fetch_add(1, Ordering::SeqCst);
                    }),
                );
                emu.mem_protect(
                    DEST_ADDR,
                    0x1000,
                    crate::cpu::instrumentation::MemPerms::READ,
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);

                unsafe { emu.run_cpu_batch(1) }.unwrap();
                assert_eq!(reads.load(Ordering::SeqCst), 0);
                assert_eq!(writes.load(Ordering::SeqCst), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn rep_insw_mmio_fallback_reads_once_per_input_word() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                use std::sync::{
                    atomic::{AtomicUsize, Ordering},
                    Arc,
                };

                const CODE_ADDR: u64 = 0x1000;
                const DEST_ADDR: u64 = 0x20_0000;
                const UNMAPPED_PORT: u64 = 0x1234;

                let reads = Arc::new(AtomicUsize::new(0));
                let writes = Arc::new(AtomicUsize::new(0));
                let read_count = Arc::clone(&reads);
                let write_count = Arc::clone(&writes);
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                emu.virt_write(CODE_ADDR, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                    .unwrap();
                emu.mmio_map(
                    DEST_ADDR,
                    2,
                    Box::new(move |_addr, _size| {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        0
                    }),
                    Box::new(move |_addr, _size, _value| {
                        write_count.fetch_add(1, Ordering::SeqCst);
                    }),
                );
                emu.reg_write(X86Reg::Rip, CODE_ADDR);
                emu.reg_write(X86Reg::Rdx, UNMAPPED_PORT);
                emu.reg_write(X86Reg::Rdi, DEST_ADDR);
                emu.reg_write(X86Reg::Rcx, 1);

                unsafe { emu.run_cpu_batch(1) }.unwrap();

                assert_eq!(reads.load(Ordering::SeqCst), 1);
                assert_eq!(writes.load(Ordering::SeqCst), 1);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    const PHASE6_FW_CFG_KEY: u16 = 0x1234;

    fn phase6_large_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }
    fn phase6_lock<T>(lock: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }


    fn phase6_flat32() -> Box<Emulator<'static, Corei7SkylakeX>> {
        Emulator::<Corei7SkylakeX>::new_with_mode(
            EmulatorConfig::default(),
            CpuSetupMode::FlatProtected32,
        )
        .unwrap()
    }

    fn phase6_prepare_fw_cfg<T: crate::cpu::instrumentation::Instrumentation>(
        emu: &mut Emulator<'_, Corei7SkylakeX, T>,
        stream: &[u8],
    ) {
        emu.devices.init(&mut emu.memory).unwrap();
        emu.device_manager
            .init(&mut emu.devices, &mut emu.memory)
            .unwrap();
        emu.device_manager.fw_cfg.add_bytes(PHASE6_FW_CFG_KEY, stream);
        emu.device_manager.fw_cfg.write_port(
            FW_CFG_IO_BASE,
            PHASE6_FW_CFG_KEY as u32,
            FW_CFG_SELECTOR_WRITE_BYTES,
            None,
            &[],
        );
    }

    fn phase6_next_fw_cfg_byte<T: crate::cpu::instrumentation::Instrumentation>(
        emu: &mut Emulator<'_, Corei7SkylakeX, T>,
    ) -> u8 {
        emu.device_manager
            .fw_cfg
            .read_port_mut(FW_CFG_DATA_PORT, FW_CFG_DATA_READ_BYTES) as u8
    }

    fn phase6_run<T: crate::cpu::instrumentation::Instrumentation>(
        emu: &mut Emulator<'_, Corei7SkylakeX, T>,
    ) {
        unsafe { emu.run_cpu_batch(1) }.unwrap();
    }

    #[cfg(feature = "instrumentation")]
    #[derive(Clone)]
    struct Phase6RepeatTrace(std::sync::Arc<std::sync::Mutex<Vec<u64>>>);

    #[cfg(feature = "instrumentation")]
    impl crate::cpu::instrumentation::Instrumentation for Phase6RepeatTrace {
        fn active_hooks(&self) -> crate::cpu::instrumentation::HookMask {
            crate::cpu::instrumentation::HookMask::EXEC
        }

        fn repeat_iteration(&mut self, rip: u64, _instr: &Instruction) {
            phase6_lock(&self.0).push(rip);
        }
    }

    #[cfg(feature = "instrumentation")]
    fn phase6_repeat_trace() -> (
        Phase6RepeatTrace,
        std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
    ) {
        let repeats = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        (Phase6RepeatTrace(std::sync::Arc::clone(&repeats)), repeats)
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn ins_byte_and_dword_prefault_before_destructive_port_read() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const DEST: u64 = 0x2000;

            for (code, width, stream) in [
                (
                    &[0xF3, 0x67, 0x6C, 0xEB, 0xFE][..],
                    1usize,
                    &[0xA1, 0xB2][..],
                ),
                (
                    &[0xF3, 0x67, 0x6D, 0xEB, 0xFE][..],
                    4usize,
                    &[0x11, 0x22, 0x33, 0x44, 0x55][..],
                ),
            ] {
                let mut emu = phase6_flat32();
                phase6_prepare_fw_cfg(&mut emu, stream);
                let port = if width == 1 {
                    FW_CFG_DATA_PORT
                } else {
                    crate::iodev::keyboard::KBD_DATA_PORT
                };
                if width == 4 {
                    emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = stream[0];
                    emu.device_manager.keyboard.kbd_controller.outb = true;
                }
                let before = vec![0xCC; width];
                emu.virt_write(CODE, code).unwrap();
                emu.mem_write(DEST, &before).unwrap();
                emu.mem_protect(
                    DEST,
                    0x1000,
                    crate::cpu::instrumentation::MemPerms::READ,
                );
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rdx, u64::from(port));
                emu.reg_write(X86Reg::Rdi, DEST);
                emu.reg_write(X86Reg::Rcx, 1);

                phase6_run(&mut emu);

                assert_eq!(emu.mem_read_vec(DEST, width).unwrap(), before);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DEST);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
                if width == 1 {
                    assert_eq!(
                        phase6_next_fw_cfg_byte(&mut emu),
                        stream[0],
                        "the faulting byte input consumed destructive fw_cfg data"
                    );
                } else {
                    assert!(
                        emu.device_manager.keyboard.kbd_controller.outb,
                        "the faulting dword input consumed the keyboard output byte"
                    );
                }

                let mut emu = phase6_flat32();
                phase6_prepare_fw_cfg(&mut emu, stream);
                if width == 4 {
                    emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = stream[0];
                    emu.device_manager.keyboard.kbd_controller.outb = true;
                }
                emu.virt_write(CODE, code).unwrap();
                emu.mem_write(DEST, &before).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rdx, u64::from(port));
                emu.reg_write(X86Reg::Rdi, DEST);
                emu.reg_write(X86Reg::Rcx, 1);
                phase6_run(&mut emu);

                let expected = if width == 1 {
                    vec![stream[0]]
                } else {
                    vec![stream[0], 0, 0, 0]
                };
                assert_eq!(emu.mem_read_vec(DEST, width).unwrap(), expected);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DEST + width as u64);
                assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
                if width == 1 {
                    assert_eq!(
                        phase6_next_fw_cfg_byte(&mut emu),
                        stream[1],
                        "the successful byte input consumed the wrong fw_cfg span"
                    );
                } else {
                    assert!(
                        !emu.device_manager.keyboard.kbd_controller.outb,
                        "the successful dword input did not consume the keyboard output byte"
                    );
                }
            }
        });
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn rep_string_io_checks_permission_once_even_when_count_zero() {
        phase6_large_stack(|| {

            const CODE: u64 = 0x1000;
            const HIGH_ZERO_ECX: u64 = 0xDEAD_BEEF_0000_0000;
            const PORT: u16 = 0x80;
            const IO_BITMAP_BASE: u16 = 0x100;
            const GP_VECTOR: u64 = 13;
            const GDT_BASE: u64 = 0x0800;
            const USER_CODE_SELECTOR: u16 = 0x001B;
            const GP_HANDLER: u64 = 0x2000;
            const IDT_BASE: u64 = 0x3000;
            const STACK_TOP: u64 = 0x5000;
            const FORMS: [&[u8]; 6] = [
                &[0xF3, 0x6C, 0xEB, 0xFE],
                &[0xF3, 0x66, 0x6D, 0xEB, 0xFE],
                &[0xF3, 0x6D, 0xEB, 0xFE],
                &[0xF3, 0x6E, 0xEB, 0xFE],
                &[0xF3, 0x66, 0x6F, 0xEB, 0xFE],
                &[0xF3, 0x6F, 0xEB, 0xFE],
            ];

            for code in FORMS {
                let mut emu = phase6_flat32();
                emu.virt_write(CODE, code).unwrap();
                emu.mem_write(GP_HANDLER, &[0xEB, 0xFE]).unwrap();
                emu.mem_write(
                    GDT_BASE + 0x18,
                    &0x00CF_FA00_0000_FFFFu64.to_le_bytes(),
                )
                .unwrap();
                let mut gate = [0u8; 8];
                gate[0..2].copy_from_slice(&(GP_HANDLER as u16).to_le_bytes());
                gate[2..4].copy_from_slice(&USER_CODE_SELECTOR.to_le_bytes());
                gate[5] = 0x8E;
                gate[6..8].copy_from_slice(&((GP_HANDLER >> 16) as u16).to_le_bytes());
                emu.mem_write(IDT_BASE + GP_VECTOR * 8, &gate).unwrap();
                emu.reg_write(X86Reg::IdtrBase, IDT_BASE);
                emu.reg_write(X86Reg::IdtrLimit, GP_VECTOR * 8 + 7);
                emu.reg_write(X86Reg::Rsp, STACK_TOP);
                // The reset task register is a valid 386 TSS at base zero.
                // Install a real denying I/O bitmap entry.  The instruction
                // must raise exactly one #GP before its zero-count exit.
                emu.mem_write(102, &IO_BITMAP_BASE.to_le_bytes()).unwrap();
                emu.mem_write(
                    u64::from(IO_BITMAP_BASE) + u64::from(PORT / 8),
                    &[0xFF, 0xFF],
                )
                .unwrap();
                emu.reg_write(X86Reg::Cs, u64::from(USER_CODE_SELECTOR));
                emu.reg_write(X86Reg::Eflags, 0x2);
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rdx, u64::from(PORT));
                emu.reg_write(X86Reg::Rcx, HIGH_ZERO_ECX);

                phase6_run(&mut emu);

                assert_eq!(
                    emu.cpu().get_exception_diag()[crate::cpu::cpu::Exception::Gp as usize],
                    1,
                    "string I/O permission must be checked once before the zero-count exit"
                );
                assert_eq!(
                    emu.reg_read(X86Reg::Rcx),
                    HIGH_ZERO_ECX,
                    "a zero 32-bit REP count must not clear high RCX"
                );
            }
        });
    }

    #[test]
    fn rep_bulk_respects_32bit_source_and_destination_segment_limits() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x3000;
            const DST: u64 = 0x5000;
            const COUNT: u64 = 3;
            const SOURCE: [u8; 12] = [
                0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x22, 0x23, 0x30, 0x31, 0x32, 0x33,
            ];

            let run_movsd = |ds_limit, es_limit| {
                let mut emu = phase6_flat32();
                emu.virt_write(CODE, &[0xF3, 0xA5, 0xEB, 0xFE]).unwrap();
                emu.mem_write(SRC, &SOURCE).unwrap();
                emu.mem_fill(DST, SOURCE.len(), 0xCC).unwrap();
                emu.cpu_mut()
                    .set_seg_for_api(X86Reg::Ds, 0x10, 0, ds_limit, false, false);
                emu.cpu_mut()
                    .set_seg_for_api(X86Reg::Es, 0x10, 0, es_limit, false, false);
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rsi, SRC);
                emu.reg_write(X86Reg::Rdi, DST);
                emu.reg_write(X86Reg::Rcx, COUNT);
                phase6_run(&mut emu);
                (
                    emu.mem_read_vec(DST, SOURCE.len()).unwrap(),
                    emu.reg_read(X86Reg::Rcx),
                    emu.reg_read(X86Reg::Rsi),
                    emu.reg_read(X86Reg::Rdi),
                )
            };

            let source_limited = run_movsd((SRC + 7) as u32, u32::MAX);
            assert_eq!(&source_limited.0[..8], &SOURCE[..8]);
            assert_eq!(&source_limited.0[8..], &[0xCC; 4]);
            assert_eq!(source_limited.1, 1);
            assert_eq!(source_limited.2, SRC + 8);
            assert_eq!(source_limited.3, DST + 8);

            let destination_limited = run_movsd(u32::MAX, (DST + 7) as u32);
            assert_eq!(&destination_limited.0[..8], &SOURCE[..8]);
            assert_eq!(&destination_limited.0[8..], &[0xCC; 4]);
            assert_eq!(destination_limited.1, 1);
            assert_eq!(destination_limited.2, SRC + 8);
            assert_eq!(destination_limited.3, DST + 8);

            let mut emu = phase6_flat32();
            emu.virt_write(CODE, &[0xF3, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.mem_fill(DST, 12, 0xCC).unwrap();
            emu.cpu_mut()
                .set_seg_for_api(X86Reg::Es, 0x10, 0, (DST + 7) as u32, false, false);
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x4433_2211);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(
                emu.mem_read_vec(DST, 12).unwrap(),
                [0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44, 0xCC, 0xCC, 0xCC, 0xCC]
            );
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DST + 8);
        });
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn rep_bulk_falls_back_for_hooks_and_page_permissions() {
        phase6_large_stack(|| {
            use crate::cpu::instrumentation::{IoHookType, MemHookType};
            use std::sync::{Arc, Mutex};

            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x2000;
            const DST: u64 = 0x3000;
            const COUNT: u64 = 3;
            const PCI_CONFIG_DATA: u16 = 0x0CFC;

            let events = Arc::new(Mutex::new(Vec::new()));
            let (trace, repeats) = phase6_repeat_trace();
            let exec_events = Arc::clone(&events);
            let mut emu =
                Emulator::<Corei7SkylakeX, Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            let _ = emu.hook_add_code(CODE..=CODE, move |_rip, _instr| {
                phase6_lock(&exec_events).push("exec");
            });
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[1, 2, 3]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(DST, 3).unwrap(), [1, 2, 3]);
            assert_eq!(phase6_lock(&events).as_slice(), ["exec"]);
            assert_eq!(phase6_lock(&repeats).len(), COUNT as usize);

            let writes = Arc::new(Mutex::new(Vec::new()));
            let observed_writes = Arc::clone(&writes);
            let mut emu = phase6_flat32();
            let _ = emu.hook_add_mem(MemHookType::Write, DST..=DST + COUNT - 1, move |ev| {
                phase6_lock(&observed_writes).push((ev.addr, ev.size));
            });
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[4, 5, 6]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(
                phase6_lock(&writes).as_slice(),
                &[(DST, 1), (DST + 1, 1), (DST + 2, 1)]
            );

            let inputs = Arc::new(Mutex::new(Vec::new()));
            let observed_inputs = Arc::clone(&inputs);
            let mut emu = phase6_flat32();
            phase6_prepare_fw_cfg(&mut emu, &[]);
            emu.device_manager.pci_conf_addr = 0x8000_0000;
            let expected_word = emu.device_manager.pci_read(PCI_CONFIG_DATA, 2) as u16;
            let word_bytes = expected_word.to_le_bytes();
            let _ = emu.hook_add_io(IoHookType::In, PCI_CONFIG_DATA..=PCI_CONFIG_DATA, move |ev| {
                phase6_lock(&observed_inputs).push((ev.port, ev.size, ev.value));
            });
            emu.virt_write(CODE, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rdx, u64::from(PCI_CONFIG_DATA));
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(
                emu.mem_read_vec(DST, 6).unwrap(),
                [
                    word_bytes[0],
                    word_bytes[1],
                    word_bytes[0],
                    word_bytes[1],
                    word_bytes[0],
                    word_bytes[1],
                ]
            );
            assert_eq!(
                phase6_lock(&inputs).as_slice(),
                &[(PCI_CONFIG_DATA, 2, u32::from(expected_word)); 3]
            );

            let mut emu = phase6_flat32();
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[0x7A, 0x7B, 0x7C]).unwrap();
            emu.mem_fill(DST, COUNT as usize, 0xCC).unwrap();
            emu.mem_protect(
                DST,
                0x1000,
                crate::cpu::instrumentation::MemPerms::READ,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DST);
            emu.reg_write(X86Reg::Rcx, COUNT);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(DST, COUNT as usize).unwrap(), [0xCC; 3]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT);
        });
    }

    #[cfg(feature = "instrumentation")]
    #[test]
    fn repeat_iteration_is_not_reported_for_faulting_element() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const FIRST_PAGE_END: u64 = 0x2FFF;
            const SECOND_PAGE: u64 = 0x3000;

            let (trace, repeats) = phase6_repeat_trace();
            let mut emu =
                Emulator::<Corei7SkylakeX, Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            emu.virt_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(0x2000, &[0x41, 0x42]).unwrap();
            emu.mem_protect(
                SECOND_PAGE,
                0x1000,
                crate::cpu::instrumentation::MemPerms::READ,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, 0x2000);
            emu.reg_write(X86Reg::Rdi, FIRST_PAGE_END);
            emu.reg_write(X86Reg::Rcx, 2);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(FIRST_PAGE_END, 1).unwrap(), [0x41]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(phase6_lock(&repeats).len(), 1);

            let (trace, repeats) = phase6_repeat_trace();
            let mut emu =
                Emulator::<Corei7SkylakeX, Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            emu.virt_write(CODE, &[0xF3, 0xAE, 0xEB, 0xFE]).unwrap();
            emu.mem_write(FIRST_PAGE_END, &[0x55]).unwrap();
            emu.mem_protect(
                SECOND_PAGE,
                0x1000,
                crate::cpu::instrumentation::MemPerms::WRITE,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x55);
            emu.reg_write(X86Reg::Rdi, FIRST_PAGE_END);
            emu.reg_write(X86Reg::Rcx, 2);
            phase6_run(&mut emu);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(emu.reg_read(X86Reg::Rdi), SECOND_PAGE);
            assert_eq!(phase6_lock(&repeats).len(), 1);

            let (trace, repeats) = phase6_repeat_trace();
            let mut emu =
                Emulator::<Corei7SkylakeX, Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            phase6_prepare_fw_cfg(&mut emu, &[0xA5, 0xB6]);
            emu.virt_write(CODE, &[0xF3, 0x6C, 0xEB, 0xFE]).unwrap();
            emu.mem_protect(
                SECOND_PAGE,
                0x1000,
                crate::cpu::instrumentation::MemPerms::READ,
            );
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rdx, u64::from(FW_CFG_DATA_PORT));
            emu.reg_write(X86Reg::Rdi, FIRST_PAGE_END);
            emu.reg_write(X86Reg::Rcx, 2);
            phase6_run(&mut emu);
            assert_eq!(emu.mem_read_vec(FIRST_PAGE_END, 1).unwrap(), [0xA5]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(phase6_lock(&repeats).len(), 1);
            assert_eq!(phase6_next_fw_cfg_byte(&mut emu), 0xB6);
        });
    }
    #[test]
    fn word_mmio_access_preserves_callback_width() {
        phase6_large_stack(|| {
            use std::sync::{Arc, Mutex};

            const CODE: u64 = 0x1000;
            const MMIO: u64 = 0x4000;

            let writes = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&writes);
            let mut emu = phase6_flat32();
            emu.mmio_map(
                MMIO,
                0x2000,
                Box::new(|_addr, _size| 0),
                Box::new(move |addr, size, value| {
                    phase6_lock(&observed).push((addr, size, value));
                }),
            );
            emu.virt_write(CODE, &[0x66, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x1234);
            emu.reg_write(X86Reg::Rdi, MMIO);
            phase6_run(&mut emu);
            assert_eq!(
                phase6_lock(&writes).as_slice(),
                &[(MMIO, 2, 0x1234)],
                "a same-page STOSW must be one width-2 memory-handler transaction"
            );

            let writes = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&writes);
            let mut emu = phase6_flat32();
            emu.mmio_map(
                MMIO,
                0x2000,
                Box::new(|_addr, _size| 0),
                Box::new(move |addr, size, value| {
                    phase6_lock(&observed).push((addr, size, value));
                }),
            );
            emu.virt_write(CODE, &[0x66, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0x1234);
            emu.reg_write(X86Reg::Rdi, MMIO + 0xFFF);
            phase6_run(&mut emu);
            assert_eq!(
                phase6_lock(&writes).as_slice(),
                &[(MMIO + 0xFFF, 1, 0x34), (MMIO + 0x1000, 1, 0x12)],
                "only a real 4 KiB crossing may split a word handler access"
            );

            let mut emu = phase6_flat32();
            emu.virt_write(MMIO, &[0x90, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, MMIO);
            phase6_run(&mut emu);
            let smc_before = emu.memory.smc_seq_next();
            emu.mmio_map(
                MMIO,
                0x1000,
                Box::new(|_addr, _size| 0),
                Box::new(|_addr, _size, _value| {}),
            );
            emu.virt_write(CODE, &[0x66, 0xAB, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rax, 0xBEEF);
            emu.reg_write(X86Reg::Rdi, MMIO);
            phase6_run(&mut emu);
            assert_eq!(
                emu.memory.smc_seq_next(),
                smc_before + 1,
                "one successful same-page word span must enqueue one SMC invalidation"
            );
        });
    }

    #[test]
    fn rep_insw_smc_preflight_consumes_only_scalar_committed_word() {
        phase6_large_stack(|| {
            const CODE_AND_DEST: u64 = 0x1000;

            let mut emu = phase6_flat32();
            phase6_prepare_fw_cfg(&mut emu, &[]);
            emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = 0xA5;
            emu.device_manager.keyboard.kbd_controller.outb = true;
            emu.virt_write(CODE_AND_DEST, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE_AND_DEST);
            emu.reg_write(
                X86Reg::Rdx,
                u64::from(crate::iodev::keyboard::KBD_DATA_PORT),
            );
            emu.reg_write(X86Reg::Rdi, CODE_AND_DEST);
            emu.reg_write(X86Reg::Rcx, 1);
            let smc_before = emu.memory.smc_seq_next();
            let io_reads_before = emu.devices.diag_io_reads;

            phase6_run(&mut emu);

            assert_eq!(emu.mem_read_vec(CODE_AND_DEST, 2).unwrap(), [0xA5, 0]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
            assert_eq!(emu.reg_read(X86Reg::Rdi), CODE_AND_DEST + 2);
            assert_eq!(
                emu.devices.diag_io_reads,
                io_reads_before + 1,
                "an SMC preflight must not read the device before scalar commit"
            );
            assert!(!emu.device_manager.keyboard.kbd_controller.outb);
            assert_eq!(emu.memory.smc_seq_next(), smc_before + 1);
        });
    }

    #[test]
    fn rep_insd_obeys_one_element_event_budget() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const DEST: u64 = 0x3000;

            let mut emu = phase6_flat32();
            phase6_prepare_fw_cfg(&mut emu, &[]);
            emu.device_manager.keyboard.kbd_controller.kbd_output_buffer = 0xA5;
            emu.device_manager.keyboard.kbd_controller.outb = true;
            emu.pc_system
                .register_timer(TimerOwner::NullTimer, 1, true, true, "phase6 insd deadline")
                .unwrap();
            emu.virt_write(CODE, &[0xF3, 0x6D, 0xEB, 0xFE]).unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(
                X86Reg::Rdx,
                u64::from(crate::iodev::keyboard::KBD_DATA_PORT),
            );
            emu.reg_write(X86Reg::Rdi, DEST);
            emu.reg_write(X86Reg::Rcx, 1);

            phase6_run(&mut emu);

            assert_eq!(emu.mem_read_vec(DEST, 4).unwrap(), [0xA5, 0, 0, 0]);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 0);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DEST + 4);
            assert!(!emu.device_manager.keyboard.kbd_controller.outb);
            assert_eq!(emu.reg_read(X86Reg::Rflags) & (1 << 16), 0);
        });
    }

    /// Bochs unit contract (string.cc fast path vs cpu.cc repeat()):
    /// the fast path charges elements to TICKS (`BX_TICKN(count-1)` →
    /// `tick_surplus`) and retires one icount per chunk, while the scalar
    /// repeat() loop retires one icount per element. Tick totals match
    /// exactly; icount deliberately differs.
    #[cfg(feature = "instrumentation")]
    #[test]
    fn fast_rep_charges_ticks_not_icount_and_matches_scalar_ticks() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x2FF8;
            const DST: u64 = 0x4FF8;
            const COUNT: u64 = 8;

            let mut fast = phase6_flat32();
            fast.pc_system
                .register_timer(TimerOwner::NullTimer, COUNT, true, true, "phase6 fast")
                .unwrap();
            fast.virt_write(CODE, &[0xF3, 0x66, 0xA5, 0xEB, 0xFE])
                .unwrap();
            fast.mem_fill(SRC, (COUNT * 2) as usize, 0x5A).unwrap();
            fast.reg_write(X86Reg::Rip, CODE);
            fast.reg_write(X86Reg::Rsi, SRC);
            fast.reg_write(X86Reg::Rdi, DST);
            fast.reg_write(X86Reg::Rcx, COUNT);
            let fast_before = fast.cpu_ref(BSP_INDEX).icount;
            let fast_surplus_before = fast.cpu_ref(BSP_INDEX).tick_surplus;
            phase6_run(&mut fast);
            let fast_retired = fast.cpu_ref(BSP_INDEX).icount - fast_before;
            let fast_surplus = fast.cpu_ref(BSP_INDEX).tick_surplus - fast_surplus_before;
            let fast_ticks = fast.ticks();

            let (trace, repeats) = phase6_repeat_trace();
            let mut scalar =
                Emulator::<Corei7SkylakeX, Phase6RepeatTrace>::new_with_mode_and_instrumentation(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                    trace,
                )
                .unwrap();
            scalar
                .pc_system
                .register_timer(TimerOwner::NullTimer, COUNT, true, true, "phase6 scalar")
                .unwrap();
            scalar
                .virt_write(CODE, &[0xF3, 0x66, 0xA5, 0xEB, 0xFE])
                .unwrap();
            scalar.mem_fill(SRC, (COUNT * 2) as usize, 0x5A).unwrap();
            scalar.reg_write(X86Reg::Rip, CODE);
            scalar.reg_write(X86Reg::Rsi, SRC);
            scalar.reg_write(X86Reg::Rdi, DST);
            scalar.reg_write(X86Reg::Rcx, COUNT);
            let scalar_before = scalar.cpu_ref(BSP_INDEX).icount;
            phase6_run(&mut scalar);
            let scalar_retired = scalar.cpu_ref(BSP_INDEX).icount - scalar_before;

            assert_eq!(fast.mem_read_vec(DST, (COUNT * 2) as usize).unwrap(), scalar.mem_read_vec(DST, (COUNT * 2) as usize).unwrap());
            assert_eq!(fast.reg_read(X86Reg::Rcx), 0);
            assert_eq!(scalar.reg_read(X86Reg::Rcx), 0);
            // Both batches execute the identical instruction tail (the REP
            // plus one parking jump), so moving element charges into
            // tick_surplus must conserve the tick total exactly while icount
            // retires per chunk (fast) vs per element (scalar repeat()).
            assert!(
                fast_surplus > 0,
                "fast path must charge elements to tick_surplus, not icount"
            );
            assert_eq!(fast_retired + fast_surplus, scalar_retired);
            assert!(fast_retired < scalar_retired);
            assert_eq!(fast_ticks, scalar.ticks());
            assert_eq!(phase6_lock(&repeats).len(), COUNT as usize);
        });
    }

    #[test]
    fn fast_rep_element_budget_uses_elements_not_bytes() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x1000;
            const SRC: u64 = 0x2000;
            const DST: u64 = 0x4000;
            const DEADLINE: u64 = 3;
            const COUNT: u64 = 7;

            for (code, width, stores) in [
                (&[0xF3, 0x66, 0xA5, 0xEB, 0xFE][..], 2u64, false),
                (&[0xF3, 0xA5, 0xEB, 0xFE][..], 4u64, false),
                (&[0xF3, 0x66, 0xAB, 0xEB, 0xFE][..], 2u64, true),
                (&[0xF3, 0xAB, 0xEB, 0xFE][..], 4u64, true),
            ] {
                let mut emu = phase6_flat32();
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, DEADLINE, true, true, "phase6 element budget")
                    .unwrap();
                emu.virt_write(CODE, code).unwrap();
                emu.mem_fill(SRC, (COUNT * width) as usize, 0x6D).unwrap();
                emu.reg_write(X86Reg::Rip, CODE);
                emu.reg_write(X86Reg::Rax, 0x1122_3344);
                emu.reg_write(X86Reg::Rsi, SRC);
                emu.reg_write(X86Reg::Rdi, DST);
                emu.reg_write(X86Reg::Rcx, COUNT);
                phase6_run(&mut emu);
                assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT - DEADLINE);
                assert_eq!(emu.reg_read(X86Reg::Rdi), DST + DEADLINE * width);
                if !stores {
                    assert_eq!(emu.reg_read(X86Reg::Rsi), SRC + DEADLINE * width);
                }
            }

            const LONG_CODE: u64 = 0x10_000;
            const LONG_SRC: u64 = 0x12_000;
            const LONG_DST: u64 = 0x14_000;
            for (code, stores) in [
                (&[0xF3, 0x48, 0xA5, 0xEB, 0xFE][..], false),
                (&[0xF3, 0x48, 0xAB, 0xEB, 0xFE][..], true),
            ] {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatLong64,
                )
                .unwrap();
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, DEADLINE, true, true, "phase6 qword budget")
                    .unwrap();
                emu.mem_write(LONG_CODE, code).unwrap();
                emu.mem_fill(LONG_SRC, (COUNT * 8) as usize, 0x7E).unwrap();
                emu.reg_write(X86Reg::Rip, LONG_CODE);
                emu.reg_write(X86Reg::Rax, 0x1122_3344_5566_7788);
                emu.reg_write(X86Reg::Rsi, LONG_SRC);
                emu.reg_write(X86Reg::Rdi, LONG_DST);
                emu.reg_write(X86Reg::Rcx, COUNT);
                phase6_run(&mut emu);
                assert_eq!(emu.reg_read(X86Reg::Rcx), COUNT - DEADLINE);
                assert_eq!(emu.reg_read(X86Reg::Rdi), LONG_DST + DEADLINE * 8);
                if !stores {
                    assert_eq!(emu.reg_read(X86Reg::Rsi), LONG_SRC + DEADLINE * 8);
                }
            }
        });
    }

    #[test]
    fn cold_tlb_rep_movsb_propagates_one_page_fault_without_committing() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x10_0000;
            const SRC: u64 = 0x10_1000;
            const DEST: u64 = 0x20_0000;
            const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;

            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                EmulatorConfig::default(),
                CpuSetupMode::FlatLong64,
            )
            .unwrap();
            emu.mem_write(CODE, &[0xF3, 0xA4, 0xEB, 0xFE]).unwrap();
            emu.mem_write(SRC, &[0x5A]).unwrap();
            emu.mem_write(SECOND_LARGE_PAGE_PDE, &0u64.to_le_bytes())
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rsi, SRC);
            emu.reg_write(X86Reg::Rdi, DEST);
            emu.reg_write(X86Reg::Rcx, 1);

            let page_faults_before = emu.cpu().get_exception_diag()[14];
            phase6_run(&mut emu);

            assert_eq!(emu.reg_read(X86Reg::Cr2), DEST);
            assert_eq!(emu.cpu().get_exception_diag()[14], page_faults_before + 1);
            assert_eq!(emu.reg_read(X86Reg::Rsi), SRC);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DEST);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
        });
    }

    #[test]
    fn cold_tlb_rep_insw_propagates_one_fault_without_consuming_port_input() {
        phase6_large_stack(|| {
            const CODE: u64 = 0x10_0000;
            const DEST: u64 = 0x20_0000;
            const SECOND_LARGE_PAGE_PDE: u64 = 0x3008;

            let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                EmulatorConfig::default(),
                CpuSetupMode::FlatLong64,
            )
            .unwrap();
            phase6_prepare_fw_cfg(&mut emu, &[0xA1, 0xB2]);
            emu.mem_write(CODE, &[0xF3, 0x66, 0x6D, 0xEB, 0xFE])
                .unwrap();
            emu.mem_write(SECOND_LARGE_PAGE_PDE, &0u64.to_le_bytes())
                .unwrap();
            emu.reg_write(X86Reg::Rip, CODE);
            emu.reg_write(X86Reg::Rdx, FW_CFG_DATA_PORT as u64);
            emu.reg_write(X86Reg::Rdi, DEST);
            emu.reg_write(X86Reg::Rcx, 1);

            let page_faults_before = emu.cpu().get_exception_diag()[14];
            phase6_run(&mut emu);

            assert_eq!(emu.reg_read(X86Reg::Cr2), DEST);
            assert_eq!(emu.cpu().get_exception_diag()[14], page_faults_before + 1);
            assert_eq!(emu.reg_read(X86Reg::Rdi), DEST);
            assert_eq!(emu.reg_read(X86Reg::Rcx), 1);
            assert_eq!(phase6_next_fw_cfg_byte(&mut emu), 0xA1);
        });
    }

    #[test]
    fn empty_memory_write_is_a_no_op() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut emu = Emulator::<Corei7SkylakeX>::new_with_mode(
                    EmulatorConfig::default(),
                    CpuSetupMode::FlatProtected32,
                )
                .unwrap();
                let before = emu.mem_read_vec(0x1000, 4).unwrap();

                emu.mem_write(0x1000, &[]).unwrap();

                assert_eq!(emu.mem_read_vec(0x1000, 4).unwrap(), before);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[cfg(feature = "std")]
    #[test]
    fn snapshot_rebuilds_runnable_and_lapic_work_masks_before_resume() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig {
                    guest_memory_size: 4 * 1024 * 1024,
                    host_memory_size: 4 * 1024 * 1024,
                    cpu_params: BxParams::default().with_topology(2, 1, 1).unwrap(),
                    ..EmulatorConfig::default()
                };
                let mut source = Emulator::<Corei7SkylakeX>::new(config.clone()).unwrap();
                source.initialize().unwrap();
                source.reset(ResetReason::Hardware).unwrap();
                source.cpu_mut_at(BSP_INDEX).activity_state = CpuActivityState::Active;
                source.cpu_mut_at(AP_INDEX).activity_state = CpuActivityState::WaitForSipi;
                source.cpu_mut_at(BSP_INDEX).lapic.timer_fired = true;
                source.runnable_mask = CpuMask::default();
                source.lapic_work_mask = CpuMask::default();

                let mut snapshot = Vec::new();
                source.save_snapshot(&mut snapshot).unwrap();

                let mut restored = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                restored.initialize().unwrap();
                restored.reset(ResetReason::Hardware).unwrap();
                restored.runnable_mask.assign(AP_INDEX, true);
                restored.lapic_work_mask.assign(AP_INDEX, true);
                restored
                    .restore_snapshot(&mut std::io::Cursor::new(snapshot))
                    .unwrap();

                assert!(restored.runnable_mask.contains(BSP_INDEX));
                assert!(!restored.runnable_mask.contains(AP_INDEX));
                assert!(restored.lapic_work_mask.contains(BSP_INDEX));
                assert!(!restored.lapic_work_mask.contains(AP_INDEX));
            })
            .unwrap()
            .join()
            .unwrap();
    }

}
