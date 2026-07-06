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
        BxCpuC, BxCpuIdTrait, CpuError, ResetReason, Result as CpuResult,
    },
    iodev::{
        devices::{DeviceManager, SystemControlPort},
        BxDevicesC,
    },
    memory::{BxMemC, BxMemoryStubC},
    params::BxParams,
    pc_system::{BxPcSystemC, TimerOwner},
    Error, Result,
};

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, format, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::AtomicBool;
#[cfg(feature = "alloc")]
use core::sync::atomic::Ordering;

const BZIMAGE_MIN_HEADER_LEN: usize = 0x264;
#[cfg(not(feature = "alloc"))]
const NO_ALLOC_MAX_AP_CPUS: usize = (crate::params::BX_MAX_SMP_THREADS_SUPPORTED as usize) - 1;
const BOCHS_SMP_QUANTUM_TICKS: u64 = 16;
const BOCHS_APIC_BUS_ID_MASK: u32 = 0xFF;
const BZIMAGE_BOOT_SIGNATURE_OFFSET: usize = 0x1FE;
const BZIMAGE_BOOT_SIGNATURE_LO: u8 = 0x55;
const BZIMAGE_BOOT_SIGNATURE_HI: u8 = 0xAA;
const BZIMAGE_HEADER_MAGIC_OFFSET: usize = 0x202;
const BZIMAGE_HEADER_MAGIC: u32 = u32::from_le_bytes(*b"HdrS");
const BZIMAGE_BOOT_VERSION_OFFSET: usize = 0x206;
const BZIMAGE_MIN_BOOT_PROTOCOL: u16 = 0x0204;

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
    /// CPU parameters
    pub cpu_params: BxParams,
    /// Enable sync=slowdown clock synchronization.
    /// When true, the emulator sleeps to match wall-clock time during active
    /// (non-HLT) execution with a GUI attached. Matches Bochs `clock: sync=slowdown`.
    /// Default: true (GUI), false (headless). Override with RUSTY_BOX_NOSYNC=1.
    pub sync_slowdown: bool,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            guest_memory_size: 32 * 1024 * 1024,
            host_memory_size: 32 * 1024 * 1024,
            memory_block_size: 128 * 1024,
            ips: 4_000_000,
            pci_enabled: true,
            cpu_params: BxParams::default(),
            sync_slowdown: false,
        }
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
/// // Access components directly for cpu_loop:
/// // emu.cpu.cpu_loop(&mut emu.memory, &[]);
/// ```
pub struct Emulator<'a, I: BxCpuIdTrait, T: Instrumentation = ()> {
    /// CPU instance (boxed because BxICache contains ~19MB fixed arrays)
    #[cfg(feature = "alloc")]
    pub cpu: alloc::boxed::Box<BxCpuC<'a, I, T>>,
    /// Application processors (CPU IDs/APIC IDs 1..N-1).
    #[cfg(feature = "alloc")]
    pub(crate) ap_cpus: Vec<alloc::boxed::Box<BxCpuC<'a, I, T>>>,
    /// CPU instance (reference for no-alloc environments)
    #[cfg(not(feature = "alloc"))]
    pub cpu: &'a mut BxCpuC<'a, I, T>,
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
    /// Memory subsystem
    pub memory: BxMemC<'a>,
    /// Device controller (I/O port handlers)
    pub devices: BxDevicesC,
    /// Device manager (actual hardware devices)
    pub device_manager: DeviceManager,
    /// PC system (timers, A20, etc.)
    pub pc_system: BxPcSystemC,
    /// Last device-visible virtual time in microseconds. Devices advance by
    /// deltas of total `pc_system.time_usec()`, matching Bochs' unified virtual
    /// clock instead of per-batch truncation/floors.
    last_device_time_usec: u64,
    /// Bochs SMP scheduler remainder from `executed %= BX_SMP_PROCESSORS`.
    smp_tick_remainder: u64,
    /// True when the last `run_cpu_batch` advanced `pc_system` internally.
    /// SMP batches tick at Bochs round boundaries so LAPIC/pc-system timers
    /// fire before the next virtual CPU slice; outer loops must not tick them
    /// a second time.
    batch_advanced_pc_system: bool,
    /// Configuration
    config: EmulatorConfig,
    /// Whether the emulator has been initialized
    initialized: bool,
    /// GUI instance (optional, can be None for headless operation)
    #[cfg(feature = "alloc")]
    gui: Option<Box<dyn BxGui>>,
    /// BIOS output file for port 0x402/0x403/0xE9 messages (std feature only)
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

    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    fn total_cpu_icount(&self) -> u64 {
        (0..self.cpu_count()).fold(0u64, |total, cpu_index| {
            total.saturating_add(self.cpu_ref(cpu_index).icount)
        })
    }

    #[cfg(feature = "alloc")]
    fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<'a, I, T> {
        if index == 0 {
            &mut self.cpu
        } else {
            &mut self.ap_cpus[index - 1]
        }
    }

    #[cfg(not(feature = "alloc"))]
    fn cpu_mut_at(&mut self, index: usize) -> &mut BxCpuC<'a, I, T> {
        if index == 0 {
            self.cpu
        } else {
            assert!(index <= self.ap_cpu_count);
            // SAFETY: &mut self guarantees no other emulator method can
            // concurrently borrow the AP CPU through this pointer.
            unsafe { &mut *self.ap_cpu_ptrs[index - 1] }
        }
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
        self.batch_advanced_pc_system = false;
        self.drain_lapic_bus();
        let batch_size = self.active_batch_step_ticks(batch_size);
        let cpu_count = self.cpu_count();
        let has_ap_cpus = cpu_count > 1;

        let mem_ptr: *mut BxMemC<'a> = &mut self.memory;
        let io_ptr = core::ptr::NonNull::from(&mut self.devices);
        let ps_ptr = core::ptr::NonNull::from(&mut self.pc_system);
        let dm_ptr = core::ptr::NonNull::from(&mut self.device_manager);
        io_ptr
            .as_ptr()
            .as_mut()
            .unwrap_unchecked()
            .set_device_manager(dm_ptr);
        let mem_static = self.mem_nonnull_static();
        io_ptr
            .as_ptr()
            .as_mut()
            .unwrap_unchecked()
            .set_mem_ptr(mem_static);
        (*dm_ptr.as_ptr()).mem_ptr = Some(mem_static);

        let pic_ref: *mut _ = &mut self.device_manager.pic;
        let dma_ref: *mut _ = &mut self.device_manager.dma;
        let mut total_elapsed_ticks = 0u64;
        let mut result: CpuResult<()> = Ok(());

        loop {
            if total_elapsed_ticks >= batch_size {
                break;
            }

            let mut runnable_count = 0usize;
            for cpu_index in 0..cpu_count {
                if self.cpu_runnable_for_batch(cpu_index) {
                    runnable_count += 1;
                }
            }

            if runnable_count == 0 {
                break;
            }

            // Bochs main.cc bx_begin_simulation: SMP scheduling engages
            // whenever BX_SMP_PROCESSORS > 1, regardless of activity states.
            // Every CPU is visited each round, the tick denominator is always
            // the full CPU count, and a CPU that executes nothing (WAIT_FOR_
            // SIPI, shutdown, idle HLT) is credited one quantum
            // ("if (n == 0) n = quantum").
            let smp = has_ap_cpus;
            let pc_tick_denominator = if smp { cpu_count as u64 } else { 1 };

            let remaining = batch_size.saturating_sub(total_elapsed_ticks);
            let per_cpu_batch = if smp {
                // Bochs SMP main.cc runs exactly one trace per CPU, and
                // icache.cc caps each SMP trace by the configured quantum
                // (default 16).
                BOCHS_SMP_QUANTUM_TICKS.min(remaining.max(1))
            } else {
                (remaining / runnable_count as u64).max(1)
            };

            let mut round_ticks = 0u64;
            for cpu_index in 0..cpu_count {
                if !self.cpu_runnable_for_batch(cpu_index) {
                    if smp {
                        // Bochs main.cc: a CPU that produced no instructions
                        // (cpu_run_trace returned immediately) is credited the
                        // SMP quantum before the round average.
                        round_ticks = round_ticks.saturating_add(BOCHS_SMP_QUANTUM_TICKS);
                    }
                    continue;
                }
                let pc_tick_offset = if smp {
                    total_elapsed_ticks.saturating_add(
                        self.smp_tick_remainder.saturating_add(round_ticks) / cpu_count as u64,
                    )
                } else {
                    0
                };
                if smp {
                    self.cpu_mut_at(cpu_index).mark_icount_sync();
                }
                let mem_extended: &'a mut BxMemC<'a> =
                    core::mem::transmute::<&mut BxMemC<'a>, &'a mut BxMemC<'a>>(&mut *mem_ptr);
                // Bochs main.cc bx_begin_simulation: in SMP mode each CPU
                // runs exactly one trace per turn (cpu_run_trace); a single
                // CPU runs the whole batch (cpu_loop).
                let slice_result = if smp {
                    self.cpu_mut_at(cpu_index).cpu_run_trace_with_io(
                        mem_extended,
                        &[],
                        per_cpu_batch,
                        pc_tick_denominator,
                        pc_tick_offset,
                        io_ptr,
                        ps_ptr,
                        Some(&mut *pic_ref),
                        Some(&mut *dma_ref),
                    )
                } else {
                    self.cpu_mut_at(cpu_index).cpu_loop_n_with_io(
                        mem_extended,
                        &[],
                        per_cpu_batch,
                        pc_tick_denominator,
                        pc_tick_offset,
                        io_ptr,
                        ps_ptr,
                        Some(&mut *pic_ref),
                        Some(&mut *dma_ref),
                    )
                };
                match slice_result {
                    Ok(executed) => {
                        let elapsed = if smp {
                            let delta = self.cpu_ref(cpu_index).icount_delta_since_sync();
                            if delta == 0 {
                                BOCHS_SMP_QUANTUM_TICKS
                            } else {
                                delta
                            }
                        } else {
                            executed
                        };
                        round_ticks = if smp {
                            round_ticks.saturating_add(elapsed)
                        } else {
                            round_ticks.max(elapsed)
                        };
                        self.drain_lapic_bus();
                    }
                    Err(err) => {
                        result = Err(err);
                        break;
                    }
                }
            }

            if result.is_err() {
                break;
            }

            // Bochs main.cc: BX_TICKN(executed / BX_SMP_PROCESSORS) at the
            // round-robin wrap, with the sub-CPU remainder carried in
            // `executed` ("executed %= BX_SMP_PROCESSORS").
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
            total_elapsed_ticks = total_elapsed_ticks.saturating_add(elapsed_ticks);
            if smp {
                // Bochs advances `bx_pc_system` after every SMP processor
                // round. Keep the Rust batch loop large for host throughput,
                // but fire pc-system/LAPIC timers at the same virtual-time
                // boundary before another CPU slice can run.
                self.advance_pc_system_after_cpu_ticks(elapsed_ticks);
                self.batch_advanced_pc_system = true;
            }

            if !smp {
                break;
            }
        }

        self.devices.clear_device_manager();
        self.devices.clear_mem_ptr();
        self.device_manager.mem_ptr = None;
        result.map(|_| total_elapsed_ticks)
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
        let mem_extended = self.borrow_memory_for_cpu();
        self.cpu
            .set_mem_bus_ptr(core::ptr::NonNull::from(&mut *mem_extended));
        let r = self.cpu.inject_external_interrupt(vector);
        self.cpu.clear_mem_bus();
        r
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

        let mut ap_cpus = Vec::with_capacity(cpu_count.saturating_sub(1) as usize);
        for cpu_id in 1..cpu_count {
            let mut ap_cpu = build_cpu()?;
            ap_cpu.configure_smp(cpu_id, topology);
            ap_cpus.push(ap_cpu);
        }
        Self::new_from_parts(config, cpu, ap_cpus)
    }

    fn new_from_parts(
        config: EmulatorConfig,
        cpu: alloc::boxed::Box<BxCpuC<'static, I, T>>,
        ap_cpus: Vec<alloc::boxed::Box<BxCpuC<'static, I, T>>>,
    ) -> Result<Box<Self>> {
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
            core::ptr::addr_of_mut!((*ptr).memory).write(memory);
            core::ptr::addr_of_mut!((*ptr).devices).write(devices);
            core::ptr::addr_of_mut!((*ptr).device_manager).write(device_manager);
            core::ptr::addr_of_mut!((*ptr).pc_system).write(pc_system);
            core::ptr::addr_of_mut!((*ptr).last_device_time_usec).write(0);
            core::ptr::addr_of_mut!((*ptr).smp_tick_remainder).write(0);
            core::ptr::addr_of_mut!((*ptr).batch_advanced_pc_system).write(false);
            core::ptr::addr_of_mut!((*ptr).config).write(config);
            core::ptr::addr_of_mut!((*ptr).initialized).write(false);
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
        for (index, ap_cpu_slot) in ap_cpus.iter_mut().take(required_ap_count).enumerate() {
            let ap_cpu: &mut BxCpuC<'a, I, T> = &mut **ap_cpu_slot;
            ap_cpu.configure_smp((index + 1) as u32, topology);
            ap_cpu_ptrs[index] = ap_cpu as *mut BxCpuC<'a, I, T>;
        }

        core::ptr::addr_of_mut!((*ptr).cpu).write(cpu);
        core::ptr::addr_of_mut!((*ptr).ap_cpu_ptrs).write(ap_cpu_ptrs);
        core::ptr::addr_of_mut!((*ptr).ap_cpu_count).write(required_ap_count);
        core::ptr::addr_of_mut!((*ptr).memory).write(memory);
        core::ptr::addr_of_mut!((*ptr).devices).write(devices);
        core::ptr::addr_of_mut!((*ptr).device_manager).write(device_manager);
        core::ptr::addr_of_mut!((*ptr).pc_system).write(pc_system);
        core::ptr::addr_of_mut!((*ptr).last_device_time_usec).write(0);
        core::ptr::addr_of_mut!((*ptr).smp_tick_remainder).write(0);
        core::ptr::addr_of_mut!((*ptr).batch_advanced_pc_system).write(false);
        core::ptr::addr_of_mut!((*ptr).config).write(config);
        core::ptr::addr_of_mut!((*ptr).initialized).write(false);
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
    #[cfg(feature = "alloc")]
    pub fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            tracing::trace!("Emulator already initialized");
            return Ok(());
        }

        tracing::debug!("Initializing emulator");

        // Step 1: Initialize PC system with IPS (line 1201)
        self.pc_system.initialize(self.config.ips);
        self.last_device_time_usec = 0;
        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        tracing::trace!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
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

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;
        // Initialize PCI bridge DRAM row boundaries from RAM size,
        // and wire PCI bridge to memory_type for immediate PAM updates.
        {
            let ramsize_mb = (self.config.guest_memory_size / (1024 * 1024)) as u32;
            self.device_manager.pci_bridge.init_dram(ramsize_mb);
            tracing::trace!("PCI bridge DRAM initialized for {}MB", ramsize_mb);
        }
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

        // Wire DMA→memory for physical DMA transfers
        let (ram_base, ram_len) = self.memory.get_ram_base_ptr();
        self.device_manager.dma.set_memory_ptrs(ram_base, ram_len);

        // Register PCI IDE BM-DMA timers (Bochs pci_ide.cc)
        {
            // Channel 0 timer
            match self.pc_system.register_timer(
                TimerOwner::PciIdeCh0,
                0,
                false,
                false,
                "PIIX IDE ch0",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[0].timer_index = Some(handle);
                    tracing::trace!("PCI IDE ch0 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch0 timer: {}", e);
                }
            }
            // Channel 1 timer
            match self.pc_system.register_timer(
                TimerOwner::PciIdeCh1,
                0,
                false,
                false,
                "PIIX IDE ch1",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[1].timer_index = Some(handle);
                    tracing::trace!("PCI IDE ch1 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch1 timer: {}", e);
                }
            }
        }

        // PIC→IOAPIC forwarding is now handled at call sites: PIC's raise/lower_irq
        // return forwarding info, and the caller (DeviceManager::tick, etc.) forwards
        // to IOAPIC. No stored pointers needed.

        // IOAPIC→PIC (ExtINT) and IOAPIC→LAPIC (interrupt delivery) are now passed
        // as parameters to service_ioapic/set_irq_level/write_aligned.
        // The MMIO callback path uses fallback stubs (no PIC/LAPIC available).

        // Register one LAPIC timer per CPU (matches Bochs per-local-APIC timers).
        for cpu_index in 0..self.cpu_count() {
            let timer_handle = self.pc_system.register_timer(
                TimerOwner::Lapic(cpu_index),
                0,     // period=0 (inactive)
                false, // continuous=false (one-shot, re-armed by periodic())
                false, // active=false
                "lapic",
            );
            match timer_handle {
                Ok(handle) => {
                    self.cpu_mut_at(cpu_index).lapic.timer_handle = Some(handle);
                    tracing::trace!(
                        "LAPIC timer registered for CPU {} with handle {}",
                        cpu_index,
                        handle
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to register LAPIC timer for CPU {}: {}",
                        cpu_index,
                        e
                    );
                }
            }
        }

        // Note: SIM->opt_plugin_ctrl("*", 0) at line 1355 unloads unused optional plugins
        // This is optional plugin management, not yet implemented in Rust version

        // Step 10: PC system register state (line 1356)
        self.pc_system.register_state();

        // Step 11: Device register state (line 1357)
        self.devices.register_state()?;
        tracing::trace!("State registered");

        // Note: bx_set_log_actions_by_device(1) at line 1359 sets up logging per device
        // This is only called if not restoring state, and is optional logging setup

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
        self.last_device_time_usec = 0;
        self.smp_tick_remainder = 0;
        self.batch_advanced_pc_system = false;
        tracing::trace!("PC system initialized with {} IPS", self.config.ips);

        // Step 2: Memory initialization (line 1312)
        // In original: BX_MEM(0)->init_memory(memSize, hostMemSize, memBlockSize);
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
        self.last_device_time_usec = 0;
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

        // Initialize device manager (actual hardware + I/O handler registration)
        self.device_manager
            .init(&mut self.devices, &mut self.memory)?;

        // Initialize PCI bridge DRAM row boundaries from RAM size.
        {
            let ramsize_mb = (self.config.guest_memory_size / (1024 * 1024)) as u32;
            self.device_manager.pci_bridge.init_dram(ramsize_mb);
            tracing::trace!("PCI bridge DRAM initialized for {}MB", ramsize_mb);
        }
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

        // Wire DMA→memory for physical DMA transfers
        let (ram_base, ram_len) = self.memory.get_ram_base_ptr();
        self.device_manager.dma.set_memory_ptrs(ram_base, ram_len);

        // Register PCI IDE BM-DMA timers (Bochs pci_ide.cc)
        {
            // Channel 0 timer
            match self.pc_system.register_timer(
                TimerOwner::PciIdeCh0,
                0,
                false,
                false,
                "PIIX IDE ch0",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[0].timer_index = Some(handle);
                    tracing::trace!("PCI IDE ch0 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch0 timer: {}", e);
                }
            }
            // Channel 1 timer
            match self.pc_system.register_timer(
                TimerOwner::PciIdeCh1,
                0,
                false,
                false,
                "PIIX IDE ch1",
            ) {
                Ok(handle) => {
                    self.device_manager.pci_ide.bmdma[1].timer_index = Some(handle);
                    tracing::trace!("PCI IDE ch1 timer registered with handle {}", handle);
                }
                Err(e) => {
                    tracing::error!("Failed to register PCI IDE ch1 timer: {}", e);
                }
            }
        }

        // PIC→IOAPIC, IOAPIC→PIC, IOAPIC→LAPIC: pointer wiring removed.
        // Forwarding is now done via parameters at call sites.

        // Register one LAPIC timer per CPU (matches Bochs per-local-APIC timers).
        for cpu_index in 0..self.cpu_count() {
            let timer_handle = self.pc_system.register_timer(
                TimerOwner::Lapic(cpu_index),
                0,     // period=0 (inactive)
                false, // continuous=false (one-shot, re-armed by periodic())
                false, // active=false
                "lapic",
            );
            match timer_handle {
                Ok(handle) => {
                    self.cpu_mut_at(cpu_index).lapic.timer_handle = Some(handle);
                    tracing::trace!(
                        "LAPIC timer registered for CPU {} with handle {}",
                        cpu_index,
                        handle
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to register LAPIC timer for CPU {}: {}",
                        cpu_index,
                        e
                    );
                }
            }
        }

        // Note: SIM->opt_plugin_ctrl("*", 0) at line 1355 unloads unused optional plugins
        // This is optional plugin management, not yet implemented in Rust version

        // Step 10: PC system register state (line 1356)
        self.pc_system.register_state();

        // Step 11: Device register state (line 1357)
        self.devices.register_state()?;
        tracing::trace!("State registered");

        // Note: bx_set_log_actions_by_device(1) at line 1359 sets up logging per device
        // This is only called if not restoring state, and is optional logging setup

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

    #[cfg(feature = "alloc")]
    /// Get reference to GUI (if set)
    pub fn gui(&self) -> Option<&(dyn BxGui + 'static)> {
        self.gui.as_deref()
    }

    /// Get mutable reference to CPU for instrumentation setup.
    pub fn cpu_mut(&mut self) -> &mut BxCpuC<'a, I, T> {
        &mut *self.cpu
    }

    #[cfg(feature = "alloc")]
    /// Update GUI with VGA text mode changes
    ///
    /// Call this periodically to refresh the display (matching vgacore.cc)
    /// Uses VGA update() function to process text mode and get update data
    pub fn update_gui(&mut self) {
        if let Some(ref mut gui) = self.gui {
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
        tracing::debug!("Emulator reset ({:?})", reset_type);

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

            // Step 3: Reset all device plugins (matches original line 406: bx_reset_plugins())
            // This resets all devices: PIC, PIT, CMOS, DMA, Keyboard, HardDrive, VGA
            self.device_manager.reset(reset_type)?;

            // Note: release_keys() at line 407 and paste.stop at line 409 not yet implemented
        }

        // Clear reset request latches. Port 92h is device state, so only a
        // hardware reset reinitializes its value/A20 latch.
        if matches!(reset_type, ResetReason::Hardware) {
            self.device_manager.port92 = SystemControlPort::new();
        } else {
            self.device_manager.port92.reset_request = None;
        }
        self.device_manager.keyboard.reset_requested = None;

        // Note: start_timers() is called separately after GUI signal handlers
        // to match original Bochs order: reset -> init_signal_handlers -> start_timers

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
            self.device_manager
                .pit
                .init_icount_sync(self.cpu.icount, ips);
            self.device_manager
                .acpi
                .init_icount_sync(self.cpu.icount, ips);
            #[cfg(feature = "std")]
            {
                self.device_manager.pit.enable_realtime_sync();
                self.device_manager.acpi.enable_realtime_sync();
            }
        }

        // Initialize VGA icount-based timing for retrace computation.
        {
            let ips = self.config.ips as u64;
            self.device_manager.vga.set_icount_sync(ips);
        }

        self.last_device_time_usec = if self.config.ips == 0 {
            0
        } else {
            self.pc_system.time_usec()
        };
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
    /// Peek at raw RAM at a physical address range (for diagnostics).
    /// Returns up to `len` bytes from the physical RAM array.
    pub fn peek_ram_at(&self, addr: usize, len: usize) -> alloc::vec::Vec<u8> {
        let ram = self.memory.ram_slice();
        if addr + len <= ram.len() {
            ram[addr..addr + len].to_vec()
        } else if addr < ram.len() {
            ram[addr..].to_vec()
        } else {
            alloc::vec::Vec::new()
        }
    }

    /// Read-only access to this emulator's configuration.
    pub fn config_ref(&self) -> &EmulatorConfig {
        &self.config
    }

    /// Check if the emulator has been initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the current system tick count
    pub fn ticks(&self) -> u64 {
        self.pc_system.time_ticks()
    }

    /// Sync A20 state from system control port to PC system and memory
    ///
    /// Call this after Port 92h writes to update A20 state throughout the system.
    pub fn sync_a20_state(&mut self) {
        self.pc_system
            .set_enable_a20(self.device_manager.port92.a20_gate);
        self.memory.set_a20_mask(self.pc_system.a20_mask());
        // Bochs pc_system.cc MemoryMappingChanged() calls BX_CPU(0)->TLB_flush()
        // after A20 changes, since A20 masking affects physical address translation.
        self.cpu.tlb_flush();
    }

    /// Process a Port 92h write
    ///
    /// This updates the A20 state and checks for reset requests.
    /// Returns true if a reset was requested.
    pub fn write_port_92h(&mut self, value: u8) -> bool {
        let a20_changed = self.device_manager.port92.write(value);

        if a20_changed {
            self.sync_a20_state();
        }

        self.device_manager.port92.reset_request.is_some()
    }

    /// Read Port 92h value
    pub fn read_port_92h(&self) -> u8 {
        self.device_manager.port92.read()
    }

    /// Check for pending reset requests (keyboard 0xFE, port 92h, PCI CF9).
    /// If a reset is pending, clears the request flags and performs that reset type.
    /// Returns true if a reset was performed.
    pub fn check_and_handle_resets(&mut self) -> Result<bool> {
        let port92_reset = self.device_manager.port92.reset_request.take();
        let keyboard_reset = self.device_manager.keyboard.reset_requested.take();
        let pci_reset = self.device_manager.pci2isa.reset_request.take();

        let reset_type = if matches!(port92_reset, Some(ResetReason::Hardware))
            || matches!(keyboard_reset, Some(ResetReason::Hardware))
            || matches!(pci_reset, Some(ResetReason::Hardware))
        {
            Some(ResetReason::Hardware)
        } else if port92_reset.is_some() || keyboard_reset.is_some() || pci_reset.is_some() {
            Some(ResetReason::Software)
        } else {
            None
        };

        if let Some(reset_type) = reset_type {
            self.reset(reset_type)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Sync A20 state if port 92h changed. Returns true if A20 was updated.
    pub fn sync_port92_a20(&mut self, last_value: &mut u8) -> bool {
        if self.device_manager.port92.value != *last_value {
            *last_value = self.device_manager.port92.value;
            self.sync_a20_state();
            true
        } else {
            false
        }
    }

    /// Set BIOS output file for port 0x402/0x403/0xE9 messages (requires std feature)
    ///
    /// When set, BIOS debug output will be written to this file instead of stdout.
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
        cylinders: u16,
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

    /// Simulate time passing (for timer-based devices)
    pub fn tick_devices(&mut self, usec: u64) {
        let icount = self.cpu.icount;
        self.device_manager.tick(usec, icount, None);
        // Process deferred ATAPI seek completion (Bochs seek_timer pattern).
        // In Bochs, start_seek() activates a timer that fires after a seek
        // delay and calls ready_to_send_atapi(). We process it here during
        // the next tick, providing the minimum 1-tick delay that separates
        // the PACKET CDB write from the data-ready interrupt.
        {
            let dm = &mut self.device_manager;
            for ch in 0..2 {
                if dm.harddrv.seek_complete_pending[ch] {
                    dm.harddrv.seek_complete_pending[ch] = false;
                    let crate::iodev::devices::DeviceManager {
                        ref mut harddrv,
                        ref mut pic,
                        ref mut pci_ide,
                        ..
                    } = *dm;
                    harddrv.ready_to_send_atapi(ch, pic, pci_ide);
                }
            }
        }
        // Process any deferred PCI port re-registrations and PAM changes
        self.device_manager
            .process_pci_deferred(&mut self.devices, &mut self.memory);
    }
    /// Advance devices to the current Bochs virtual time.
    #[inline]
    pub fn advance_devices_to_pc_time(&mut self) {
        if self.config.ips == 0 {
            return;
        }
        let now = self.pc_system.time_usec();
        let delta = now.wrapping_sub(self.last_device_time_usec);
        #[cfg(feature = "std")]
        let realtime_timer_due = self.device_manager.pit.realtime_sync_enabled()
            || self.device_manager.acpi.realtime_sync_enabled();
        #[cfg(not(feature = "std"))]
        let realtime_timer_due = false;

        if delta == 0 && !realtime_timer_due {
            return;
        }
        if realtime_timer_due || delta >= 1_000 || self.device_manager.keyboard.needs_fast_service()
        {
            self.tick_devices(delta);
            self.last_device_time_usec = now;
        }
    }

    #[inline]
    fn advance_pc_system_after_cpu_ticks(&mut self, ticks: u64) {
        self.pc_system.tickn(ticks as u32);
        self.dispatch_timer_fires();
        self.advance_devices_to_pc_time();
        self.sync_event_flags();
    }

    #[inline]
    fn millisecond_timer_quantum_ticks(&self) -> Option<u64> {
        const BOCHS_WAIT_STEP_TICKS: u64 = 10;
        const DEVICE_QUANTUM_USEC: u64 = 1_000;

        let ips = self.config.ips as u64;
        if ips == 0 {
            return None;
        }

        Some((ips * DEVICE_QUANTUM_USEC / 1_000_000).clamp(BOCHS_WAIT_STEP_TICKS, u32::MAX as u64))
    }

    /// Active CPU batches yield every ~1 ms so pc_system timer fires, LAPIC
    /// events, GUI status, and device time are serviced promptly. Unlike HLT
    /// waits, this deliberately does not clamp to the next pc_system countdown:
    /// a one-tick LAPIC timer would otherwise collapse throughput to trace-sized
    /// batches.
    #[inline]
    fn active_batch_step_ticks(&self, requested: u64) -> u64 {
        const BIOS_POLL_SAFE_ACTIVE_BATCH_TICKS: u64 = 4_096;

        match self.millisecond_timer_quantum_ticks() {
            Some(quantum) => requested
                .min(quantum)
                .min(BIOS_POLL_SAFE_ACTIVE_BATCH_TICKS),
            None => requested,
        }
    }

    /// HLT/MWAIT wait-loop tick quantum.
    ///
    /// Bochs advances halted CPUs with repeated `BX_TICKN(10)`, but our
    /// usec-driven devices are outside `pc_system` and are expensive to tick
    /// hundreds of thousands of times per virtual second. Advance up to a
    /// 1 ms quantum, while still stopping at the next `pc_system` timer so
    /// LAPIC and other registered timers fire at their exact tick boundary.
    #[inline]
    fn hlt_wait_step_ticks(&self) -> u32 {
        const BOCHS_WAIT_STEP_TICKS: u32 = 10;

        let ticks_until_pc_event = self.pc_system.get_num_cpu_ticks_left_next_event().max(1);
        let Some(quantum_ticks) = self.millisecond_timer_quantum_ticks() else {
            return BOCHS_WAIT_STEP_TICKS.min(ticks_until_pc_event);
        };

        (quantum_ticks as u32).min(ticks_until_pc_event)
    }

    /// Dispatch timer fires accumulated by `pc_system.tickn()`.
    ///
    /// `countdown_event` records fired timer owners instead of calling fn ptrs.
    /// This method drains them and performs the device-specific action.
    pub fn dispatch_timer_fires(&mut self) {
        let (owners, counts, count) = self.pc_system.take_fired_timers();
        for entry in 0..count {
            match owners[entry] {
                TimerOwner::NullTimer => {}
                TimerOwner::PciIdeCh0 => {
                    for _ in 0..counts[entry] {
                        self.device_manager.pci_ide.timer(0);
                    }
                }
                TimerOwner::PciIdeCh1 => {
                    for _ in 0..counts[entry] {
                        self.device_manager.pci_ide.timer(1);
                    }
                }
                TimerOwner::Lapic(cpu_index) => {
                    if cpu_index < self.cpu_count() {
                        self.cpu_mut_at(cpu_index).lapic.timer_fired = true;
                    }
                }
            }
        }
    }

    #[cfg(feature = "alloc")]
    fn drain_lapic_bus(&mut self) {
        let cpu_count = self.cpu_count();
        for src in 0..cpu_count {
            while let Some(ipi) = { self.cpu_mut_at(src).lapic.take_pending_ipi() } {
                self.deliver_pending_ipi(cpu_count, src, ipi);
            }
        }
    }

    #[cfg(not(feature = "alloc"))]
    fn drain_lapic_bus(&mut self) {
        let cpu_count = self.cpu_count();
        for src in 0..cpu_count {
            while let Some(ipi) = { self.cpu_mut_at(src).lapic.take_pending_ipi() } {
                self.deliver_pending_ipi(cpu_count, src, ipi);
            }
        }
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
        reactivate_from_previous_fire: bool,
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
                let result = if reactivate_from_previous_fire {
                    self.pc_system
                        .reactivate_timer_relative(handle, activation.delay_ticks)
                } else {
                    self.pc_system
                        .activate_timer(handle, activation.delay_ticks, false)
                };
                if let Err(e) = result {
                    tracing::error!(
                        "CPU {cpu_index} LAPIC timer activate (handle {handle}) failed: {e:?}"
                    );
                }
            }
            if activation.update_ticks_initial {
                let ticks_now = self.pc_system.time_ticks();
                self.cpu_mut_at(cpu_index)
                    .lapic
                    .set_ticks_initial(ticks_now);
            }
        }
    }

    fn service_lapic_local_events(&mut self) {
        for cpu_index in 0..self.cpu_count() {
            while let Some(cpu_event) = self.cpu_mut_at(cpu_index).lapic.take_pending_cpu_event() {
                self.apply_lapic_cpu_event(cpu_index, Some(cpu_event));
            }

            while self.cpu_ref(cpu_index).lapic.timer_fired {
                let ticks_now = self.pc_system.time_ticks();
                let (timer_handle, deactivate, activate) = {
                    let cpu = self.cpu_mut_at(cpu_index);
                    cpu.lapic.current_ticks = ticks_now;
                    cpu.lapic.ticks_at_sync = ticks_now;
                    cpu.lapic.icount_at_sync = cpu.icount;
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

                self.pc_system.tickn(0);
                self.dispatch_timer_fires();
            }

            let ticks_now = self.pc_system.time_ticks();
            let (timer_handle, deactivate, activate, eoi_vector) = {
                let cpu = self.cpu_mut_at(cpu_index);
                cpu.lapic.current_ticks = ticks_now;
                cpu.lapic.ticks_at_sync = ticks_now;
                cpu.lapic.icount_at_sync = cpu.icount;

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
                let mem_ptr =
                    core::ptr::NonNull::from(&mut *unsafe { self.borrow_memory_for_cpu() });
                let cpu = self.cpu_mut_at(target);
                cpu.set_mem_bus_ptr(mem_ptr);
                cpu.deliver_sipi(vector);
                cpu.clear_mem_bus();
            }
        }
    }
    /// Synchronize device event flags to CPU event fields.
    ///
    /// PIC, LAPIC, and pc_system set boolean flags when they need to
    /// signal the CPU. This method reads those flags, applies the
    /// corresponding bits to `cpu.pending_event` / `cpu.async_event`,
    /// and clears the flags.
    pub fn sync_event_flags(&mut self) {
        self.drain_lapic_bus();
        self.service_lapic_local_events();
        // PIC: BX_RAISE_INTR
        if self.device_manager.pic.irq_pending {
            self.cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
            self.device_manager.pic.irq_pending = false;
        }
        // PIC: BX_CLEAR_INTR
        if self.device_manager.pic.irq_cleared {
            self.cpu.clear_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
            self.device_manager.pic.irq_cleared = false;
        }
        // IOAPIC: drain pending deliveries to LAPICs
        {
            let (deliveries, count) = self.device_manager.ioapic.take_pending_deliveries();
            if count > 0 {
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
            }
        }
        self.drain_lapic_bus();
        self.service_lapic_local_events();
        // LAPIC: BX_EVENT_PENDING_LAPIC_INTR
        for cpu_index in 0..self.cpu_count() {
            let cpu = self.cpu_mut_at(cpu_index);
            if cpu.lapic.intr_pending {
                cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_LAPIC_INTR);
                cpu.lapic.intr_pending = false;
            }
        }
        // pc_system: raise_intr
        if self.pc_system.intr_raised {
            self.cpu.signal_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
            self.pc_system.intr_raised = false;
        }
        // pc_system: clear_intr
        if self.pc_system.intr_cleared {
            self.cpu.clear_event(BxCpuC::<I>::BX_EVENT_PENDING_INTR);
            self.pc_system.intr_cleared = false;
        }
        // pc_system: async_event (HRQ/timer)
        if self.pc_system.async_event_pending {
            self.cpu.async_event = 1;
            self.pc_system.async_event_pending = false;
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
        self.memory.load_RAM(&gdt_bytes, GDT_ADDR)?;

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
            self.memory.load_RAM(initrd_data, initrd_load_addr)?;

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
        self.memory.load_RAM(&boot_params, boot_params_addr)?;

        // =====================================================================
        // Write command line
        // =====================================================================
        let cmdline_bytes = cmdline.as_bytes();
        let cmdline_len = core::cmp::min(cmdline_bytes.len(), 2047);
        let mut cmdline_buf = alloc::vec![0u8; cmdline_len + 1]; // null-terminated
        cmdline_buf[..cmdline_len].copy_from_slice(&cmdline_bytes[..cmdline_len]);
        self.memory.load_RAM(&cmdline_buf, cmdline_addr)?;
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
            self.memory.load_RAM(&madt, MADT_ADDR)?;

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
            self.memory.load_RAM(&xsdt, XSDT_ADDR)?;

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
            self.memory.load_RAM(&rsdp, RSDP_ADDR)?;

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
        self.memory.load_RAM(pm_kernel, code32_start as u64)?;

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

        // Verify VGA BIOS ROM is accessible
        {
            // Check through ROM area (not RAM)
            let rom_bytes = self.memory.peek_ram(0xC0000, 4);
            tracing::trace!(
                "VGA ROM check via peek_ram(0xC0000): {:02X?} (expect [55, AA, ...])",
                rom_bytes
            );
            // Also verify IPL table area is writable
            let ipl_bytes = self.memory.peek_ram(0x9FF00, 4);
            tracing::trace!(
                "IPL table check at 0x9FF00: {:02X?} (expect zeros before POST)",
                ipl_bytes
            );
            // Check total memory size
            tracing::trace!("Memory len={:#x}", self.memory.get_memory_len());
        }

        // Force initial GUI update to show initial state
        self.device_manager.vga.force_initial_update();
        self.update_gui(); // Force initial update

        let mut instructions_executed = 0u64;
        #[cfg(feature = "std")]
        let mut slowdown_start = std::time::Instant::now();
        #[cfg(feature = "std")]
        let mut slowdown_ticks_base = self.pc_system.time_ticks();
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
        let mut last_port92_value: u8 = self.device_manager.port92.value;

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
            // 1. Handle GUI events (keyboard input) - do this first to avoid borrow conflicts

            let mut scancodes_to_send = Vec::new();
            let mut serial_input = Vec::new();
            if let Some(ref mut gui) = self.gui {
                gui.handle_events();
                scancodes_to_send = gui.get_pending_scancodes();
                serial_input = gui.get_pending_serial_input();
            }

            // Send scancodes to keyboard device
            for scancode in scancodes_to_send {
                self.device_manager.keyboard.send_scancode(scancode);
            }

            // Send serial input to COM1 (ttyS0)
            for byte in serial_input {
                self.device_manager.serial.receive_byte(0, byte);
            }

            // 2. Execute CPU instructions in batches
            let batch_size = (max_instructions - instructions_executed).min(INSTRUCTION_BATCH_SIZE);
            // SAFETY: see borrow_memory_for_cpu / run_cpu_batch
            let result = unsafe { self.run_cpu_batch(batch_size) };

            // Apply PAM register changes immediately (BIOS needs this before next batch)
            if self.device_manager.pam_needs_update {
                self.device_manager
                    .process_pci_deferred(&mut self.devices, &mut self.memory);
            }

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

                    // Port 92h (System Control) may have changed A20 during execution.
                    // Sync PC system + memory masks if any writes occurred.
                    if self.device_manager.port92.value != last_port92_value {
                        last_port92_value = self.device_manager.port92.value;
                        self.sync_a20_state();
                    }

                    // Check for reset requests: Port 92h, keyboard 0xFE, or PCI CF9
                    let port92_reset = self.device_manager.port92.reset_request.take();
                    let keyboard_reset = self.device_manager.keyboard.reset_requested.take();
                    let pci_reset = self.device_manager.pci2isa.reset_request.take();
                    let reset_type = if matches!(port92_reset, Some(ResetReason::Hardware))
                        || matches!(keyboard_reset, Some(ResetReason::Hardware))
                        || matches!(pci_reset, Some(ResetReason::Hardware))
                    {
                        Some(ResetReason::Hardware)
                    } else if port92_reset.is_some()
                        || keyboard_reset.is_some()
                        || pci_reset.is_some()
                    {
                        Some(ResetReason::Software)
                    } else {
                        None
                    };
                    if let Some(reset_type) = reset_type {
                        if port92_reset.is_some() {
                            #[cfg(feature = "std")]
                            log_reset(&format!(
                                "PORT 92h FAST {:?} RESET at RIP={:#x} icount={}",
                                reset_type,
                                self.cpu.rip(),
                                self.cpu.icount
                            ));
                        }
                        if keyboard_reset.is_some() {
                            #[cfg(feature = "std")]
                            log_reset(&format!(
                                "KEYBOARD {:?} RESET at RIP={:#x} icount={}",
                                reset_type,
                                self.cpu.rip(),
                                self.cpu.icount
                            ));
                        }
                        if let Some(pci_reset) = pci_reset {
                            #[cfg(feature = "std")]
                            log_reset(&format!(
                                "PCI CF9 {:?} RESET at RIP={:#x} icount={}",
                                pci_reset,
                                self.cpu.rip(),
                                self.cpu.icount
                            ));
                        }
                        self.reset(reset_type)?;
                        last_port92_value = self.device_manager.port92.value;
                        continue;
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
                        let mem = self.memory.ram_slice();
                        let ipl_count = if 0x9FF81 < mem.len() {
                            u16::from_le_bytes([mem[0x9FF80], mem[0x9FF81]])
                        } else {
                            0
                        };
                        let ipl0_type = if 0x9FF01 < mem.len() {
                            u16::from_le_bytes([mem[0x9FF00], mem[0x9FF01]])
                        } else {
                            0
                        };
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
                                let mem_peek = self.memory.ram_slice();
                                let bp2 = if bp_phys + 3 < mem_peek.len() {
                                    u16::from_le_bytes([
                                        mem_peek[bp_phys + 2],
                                        mem_peek[bp_phys + 3],
                                    ])
                                } else {
                                    0
                                };
                                let bp4 = if bp_phys + 5 < mem_peek.len() {
                                    u16::from_le_bytes([
                                        mem_peek[bp_phys + 4],
                                        mem_peek[bp_phys + 5],
                                    ])
                                } else {
                                    0
                                };
                                let bp6 = if bp_phys + 7 < mem_peek.len() {
                                    u16::from_le_bytes([
                                        mem_peek[bp_phys + 6],
                                        mem_peek[bp_phys + 7],
                                    ])
                                } else {
                                    0
                                };
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
                            #[cfg(feature = "std")]
                            let hlt_wall_start = std::time::Instant::now();
                            while hlt_budget < 100_000_000 {
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
                                self.pc_system.tickn(step);
                                self.dispatch_timer_fires();
                                hlt_budget += step as u64;
                                self.advance_devices_to_pc_time();
                                self.sync_event_flags();
                                if !self.can_fast_forward_bsp_hlt() {
                                    break;
                                }
                                // Wall-clock throttle: sleep if virtual time races ahead
                                #[cfg(feature = "std")]
                                {
                                    let virtual_usec =
                                        hlt_budget * 1_000_000 / (self.config.ips as u64).max(1);
                                    let wall_usec = hlt_wall_start.elapsed().as_micros() as u64;
                                    if self.config.sync_slowdown && virtual_usec > wall_usec + 1_000
                                    {
                                        let sleep_usec = (virtual_usec - wall_usec).min(15_000);
                                        std::thread::sleep(std::time::Duration::from_micros(
                                            sleep_usec,
                                        ));
                                    }
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
                                let batch2 = (max_instructions
                                    .saturating_sub(instructions_executed))
                                .min(INSTRUCTION_BATCH_SIZE);
                                if batch2 == 0 {
                                    break;
                                }
                                // SAFETY: see borrow_memory_for_cpu / run_cpu_batch
                                let r2 = unsafe { self.run_cpu_batch(batch2) };
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
                                    let s = self.hlt_wait_step_ticks();
                                    self.pc_system.tickn(s);
                                    self.dispatch_timer_fires();
                                    hlt2 += s as u64;
                                    self.advance_devices_to_pc_time();
                                    self.sync_event_flags();
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

                    // Propagate A20 gate changes from keyboard controller to memory system
                    // Matching Bochs BX_SET_ENABLE_A20() which immediately updates pc_system and memory
                    if self.device_manager.keyboard.a20_change_pending {
                        self.device_manager.keyboard.a20_change_pending = false;
                        let a20 = self.device_manager.keyboard.a20_enabled;
                        self.pc_system.set_enable_a20(a20);
                        self.memory.set_a20_mask(self.pc_system.a20_mask());
                        // Bochs pc_system.cc MemoryMappingChanged() calls BX_CPU(0)->TLB_flush()
                        // after A20 changes, since A20 masking affects physical address translation.
                        self.cpu.tlb_flush();
                    }

                    // Log batch sizes and check if timer ticking works
                    #[cfg(debug_assertions)]
                    if instructions_executed < 5 * INSTRUCTION_BATCH_SIZE
                        || instructions_executed % 100_000 < INSTRUCTION_BATCH_SIZE
                    {
                        let pit_c0_count = self.device_manager.pit.counters[0].count;
                        // Read BDA timer tick counter at 0x046C (4 bytes) directly from RAM
                        let bda_ticks = {
                            let (ptr, len) = self.memory.get_raw_memory_ptr();
                            if 0x046C + 4 <= len {
                                // SAFETY: pointer and length validated by caller; memory region is valid
                                unsafe {
                                    let p = ptr.add(0x046C) as *const u32;
                                    *p
                                }
                            } else {
                                0
                            }
                        };
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

            // 5. sync=slowdown: interval-based throttle matching Bochs slowdown.cc.
            // Compares emulated vs wall-clock time over a sliding 1-second window.
            // Resets the window periodically to prevent unbounded deficit accumulation
            // (which would cause massive sleeps when transitioning from active to idle).
            #[cfg(feature = "std")]
            if self.config.sync_slowdown && self.config.ips > 0 {
                let wall_elapsed = slowdown_start.elapsed().as_micros() as u64;
                // Reset window every 1 second to prevent deficit accumulation
                if wall_elapsed > 1_000_000 {
                    slowdown_start = std::time::Instant::now();
                    slowdown_ticks_base = self.pc_system.time_ticks();
                } else {
                    let delta_ticks = self
                        .pc_system
                        .time_ticks()
                        .saturating_sub(slowdown_ticks_base);
                    let emu_usec = delta_ticks.saturating_mul(1_000_000) / (self.config.ips as u64);
                    // Sleep if emulated time is >50ms ahead within this window.
                    // 50ms threshold avoids Windows 15.6ms timer granularity issues.
                    if emu_usec > wall_elapsed + 50_000 {
                        let sleep_usec = (emu_usec - wall_elapsed).min(50_000);
                        std::thread::sleep(std::time::Duration::from_micros(sleep_usec));
                    }
                }
            }

            // 6. Check if we should exit (e.g., shutdown requested)
            // TODO: Add shutdown flag check
        }

        tracing::trace!(
            "Interactive execution completed: {} instructions",
            instructions_executed
        );

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
            // icount = instruction count (REP iterations count as separate ticks)
            let bochs_ticks = self.cpu.icount;
            tracing::debug!("[PERF] dispatches={pi} bochs_ticks={bochs_ticks} tlb_hit={tlb_h} tlb_miss={tlb_m} tlb_hit%={tlb_pct:.2}% page_walks={pw}");
        }

        Ok(instructions_executed)
    }

    #[cfg(feature = "alloc")]
    /// Execute a batch of instructions cooperatively (no blocking loop).
    ///
    /// Designed for single-threaded environments like WASM where the caller
    /// must yield control back to the event loop regularly. Runs up to
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
                #[cfg(feature = "std")]
                let hlt_wall_start = std::time::Instant::now();
                while hlt_budget < 100_000_000 {
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
                    self.pc_system.tickn(step);
                    self.dispatch_timer_fires();
                    hlt_budget += step as u64;
                    self.advance_devices_to_pc_time();
                    self.sync_event_flags();
                    if !self.can_fast_forward_bsp_hlt() {
                        break;
                    }
                    // Wall-clock throttle: sleep if virtual time races ahead
                    #[cfg(feature = "std")]
                    {
                        let virtual_usec = hlt_budget * 1_000_000 / ips.max(1);
                        let wall_usec = hlt_wall_start.elapsed().as_micros() as u64;
                        if self.config.sync_slowdown && virtual_usec > wall_usec + 1_000 {
                            let sleep_usec = (virtual_usec - wall_usec).min(15_000);
                            std::thread::sleep(std::time::Duration::from_micros(sleep_usec));
                        }
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
                    tracing::warn!(
                        "PIC interrupt injection (vector {vector:#04x}) failed: {e:?}"
                    );
                }
            }

            // --- Tight loop: if CPU was woken from MWAIT and wall budget remains,
            // run another cycle instead of returning to egui event loop.
            // This matches Bochs's dedicated CPU thread which never yields to GUI.
            if matches!(self.cpu.activity_state, CpuActivityState::Active) && {
                #[cfg(feature = "std")]
                {
                    wall_start.elapsed() < wall_budget
                }
                #[cfg(not(feature = "std"))]
                {
                    true
                }
            } {
                continue 'batch;
            }

            break 'batch;
        }

        // Sync A20 state
        self.sync_a20_state();

        // Handle keyboard scancodes and serial input from GUI
        let mut scancodes_to_send = Vec::new();
        let mut serial_input = Vec::new();
        if let Some(ref mut gui) = self.gui {
            gui.handle_events();
            scancodes_to_send = gui.get_pending_scancodes();
            serial_input = gui.get_pending_serial_input();
        }
        for scancode in scancodes_to_send {
            self.device_manager.keyboard.send_scancode(scancode);
        }
        for byte in serial_input {
            self.device_manager.serial.receive_byte(0, byte);
        }

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
        cylinders: u16,
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
        cylinders: u16,
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

    /// Get mem_host_base pointer value for diagnostics.
    pub fn get_mem_host_base(&self) -> u64 {
        self.cpu.mem_host_base as u64
    }

    /// Get mem_host_len for diagnostics.
    pub fn get_mem_host_len(&self) -> usize {
        self.cpu.mem_host_len
    }

    /// Read a physical dword directly from host memory (bypassing TLB/paging).
    /// Returns None if address is out of range.
    pub fn read_phys_dword(&self, paddr: u64) -> Option<u32> {
        let addr = paddr as usize;
        let host_base = self.cpu.mem_host_base;
        if !host_base.is_null() && addr + 4 <= self.cpu.mem_host_len {
            // SAFETY: host pointer validated during TLB fill; offset within page bounds
            Some(unsafe { (host_base.add(addr) as *const u32).read_unaligned() })
        } else {
            None
        }
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
        // Device tick diagnostics
        tracing::trace!("--- Device Tick Diag ---");
        tracing::trace!(
            "  tick_count={} total_usec={} pit_fires={} irq0_latched={} iac_count={}",
            self.device_manager.diag_tick_count,
            self.device_manager.diag_total_usec,
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
        // Dump key code addresses from memory
        {
            let ram = self.memory.ram_slice();
            let addrs: &[(u64, &str)] = &[
                (0x01e1d340, "delay_loop_entry"),
                (0x01e38ef0, "jmp_target_after_delay"),
                (0x01207430, "outer_loop_context"),
                (0x01207460, "stack_ret_addr_1"),
                (0x012074e0, "stack_ret_addr_2"),
            ];
            for (paddr, label) in addrs {
                let p = *paddr as usize;
                if p + 48 <= ram.len() {
                    let code = &ram[p..p + 48];
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
        // Dump stack (16 qwords)
        let rsp = self.cpu.rsp();
        if rsp > 0xffffffff80000000 {
            let cr3 = self.cpu.cr3 & !0xFFF;
            let ram = self.memory.ram_slice();
            let ram_len = ram.len();
            let read_u64 = |addr: u64| -> u64 {
                let pml4_idx = (addr >> 39) & 0x1FF;
                let pdpt_idx = (addr >> 30) & 0x1FF;
                let pd_idx = (addr >> 21) & 0x1FF;
                let pt_idx = (addr >> 12) & 0x1FF;
                let page_off = addr & 0xFFF;
                let safe_read = |phys: u64| -> u64 {
                    let off = phys as usize;
                    if off + 8 > ram_len {
                        return 0;
                    }
                    u64::from_le_bytes(
                        ram[off..off + 8]
                            .try_into()
                            .expect("8-byte slice converts to [u8; 8]"),
                    )
                };
                let pml4e = safe_read(cr3 + pml4_idx * 8);
                if pml4e & 1 == 0 {
                    return 0;
                }
                let pdpte = safe_read((pml4e & 0xFFFFF_FFFFF000) + pdpt_idx * 8);

                if pdpte & 1 == 0 {
                    return 0;
                }
                if pdpte & 0x80 != 0 {
                    return safe_read((pdpte & 0xFFFFF_C0000000) | (addr & 0x3FFFFFFF));
                }
                let pde = safe_read((pdpte & 0xFFFFF_FFFFF000) + pd_idx * 8);
                if pde & 1 == 0 {
                    return 0;
                }
                if pde & 0x80 != 0 {
                    return safe_read((pde & 0xFFFFF_FFE00000) | (addr & 0x1FFFFF));
                }
                let pte = safe_read((pde & 0xFFFFF_FFFFF000) + pt_idx * 8);
                if pte & 1 == 0 {
                    return 0;
                }
                safe_read((pte & 0xFFFFF_FFFFF000) | page_off)
            };
            tracing::trace!("--- Stack at RSP={:#018x} ---", rsp);
            for i in 0..16 {
                let addr = rsp.wrapping_add(i * 8);
                let val = read_u64(addr);
                let marker = if val > 0xffffffff81000000 && val < 0xffffffff82000000 {
                    " <-- kernel text?"
                } else {
                    ""
                };
                tracing::trace!("  [{:+4}] {:#018x}{}", i * 8, val, marker);
            }
        }
        // Dump 64 bytes of code at current RIP via manual page walk
        let rip = self.cpu.rip();
        if rip > 0xffffffff80000000 {
            let cr3 = self.cpu.cr3 & !0xFFF;
            let ram = self.memory.ram_slice();
            let read_u64 = |paddr: u64| -> u64 {
                let p = paddr as usize;
                if p + 8 <= ram.len() {
                    u64::from_le_bytes(
                        ram[p..p + 8]
                            .try_into()
                            .expect("8-byte slice converts to [u8; 8]"),
                    )
                } else {
                    0
                }
            };
            let pml4_idx = (rip >> 39) & 0x1FF;
            let pdpt_idx = (rip >> 30) & 0x1FF;
            let pd_idx = (rip >> 21) & 0x1FF;
            let pt_idx = (rip >> 12) & 0x1FF;
            let pml4e = read_u64(cr3 + pml4_idx * 8);
            if pml4e & 1 != 0 {
                let pdpte = read_u64((pml4e & 0x000FFFFF_FFFFF000) + pdpt_idx * 8);
                if pdpte & 1 != 0 {
                    let paddr = if pdpte & 0x80 != 0 {
                        (pdpte & 0x000FFFFF_C0000000) | (rip & 0x3FFFFFFF)
                    } else {
                        let pde = read_u64((pdpte & 0x000FFFFF_FFFFF000) + pd_idx * 8);
                        if pde & 1 != 0 {
                            if pde & 0x80 != 0 {
                                (pde & 0x000FFFFF_FFE00000) | (rip & 0x1FFFFF)
                            } else {
                                let pte = read_u64((pde & 0x000FFFFF_FFFFF000) + pt_idx * 8);
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
                    if paddr != 0 && (paddr as usize) + 64 <= ram.len() {
                        let code = &ram[paddr as usize..(paddr as usize) + 64];
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
    use crate::cpu::instrumentation::{CpuSetupMode, X86Reg};
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
        bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24);
        bsp.lapic.write_aligned(
            ICR_LOW,
            ((crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8)
                | ICR_LEVEL_ASSERT
                | ICR_TRIGGER_LEVEL,
        );
        bsp.lapic.write_aligned(
            ICR_LOW,
            (crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8 | ICR_TRIGGER_LEVEL,
        );
    }

    fn send_bsp_icr_sipi(emu: &mut Emulator<'_, Corei7SkylakeX, ()>, vector: u8) {
        let bsp = emu.cpu_mut_at(BSP_INDEX);
        bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24);
        bsp.lapic.write_aligned(
            ICR_LOW,
            vector as u32
                | ((crate::cpu::apic::ApicDeliveryMode::Sipi as u32) << 8)
                | ICR_LEVEL_ASSERT,
        );
    }

    fn read_fw_cfg_u16(fw_cfg: &mut crate::iodev::fw_cfg::BxFwCfg, key: u16) -> u16 {
        fw_cfg.write_port(
            FW_CFG_IO_BASE,
            key as u32,
            FW_CFG_SELECTOR_WRITE_BYTES,
            None,
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
    fn hlt_wait_batches_device_ticks_even_when_pc_timer_is_due_each_tick() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.pc_system
                    .register_timer(TimerOwner::NullTimer, 1, true, true, "one_tick")
                    .unwrap();

                let mut hlt_budget = 0u64;
                while hlt_budget < emu.config.ips as u64 / 1_000 {
                    let step = emu.hlt_wait_step_ticks();
                    emu.pc_system.tickn(step);
                    emu.dispatch_timer_fires();
                    hlt_budget += step as u64;
                    emu.advance_devices_to_pc_time();
                }

                assert_eq!(emu.device_manager.diag_total_usec, 1_000);
                assert!(
                    emu.device_manager.diag_tick_count <= 1,
                    "device ticks should be batched to 1ms, got {} calls",
                    emu.device_manager.diag_tick_count
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(feature = "std")]
    #[test]
    fn realtime_pit_service_runs_before_virtual_millisecond_delta() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let mut config = EmulatorConfig::default();
                config.ips = 300_000_000;
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.pc_system.initialize(emu.config.ips);
                emu.device_manager
                    .pit
                    .init_icount_sync(emu.cpu.icount, emu.config.ips as u64);
                emu.device_manager.pit.enable_realtime_sync();

                let before = emu.device_manager.pit.total_ticks;
                emu.pc_system.tickn(4_096);
                std::thread::sleep(std::time::Duration::from_millis(10));
                emu.advance_devices_to_pc_time();

                assert!(
                    emu.device_manager.pit.total_ticks > before,
                    "realtime PIT must be serviced even when configured IPS keeps virtual delta below 1 ms"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn run_cpu_batch_does_not_collapse_to_trace_sized_batches_when_timer_is_due_each_tick() {
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
                    executed >= 1024,
                    "near timer collapsed CPU batch to {executed} instructions"
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

    #[test]
    fn active_cpu_batch_returns_at_millisecond_quantum_for_timer_service() {
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
                    (1_024..10_000).contains(&executed),
                    "active batch should yield near the 1ms timer-service quantum, got {executed}"
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

                assert_eq!(high_ips_emu.active_batch_step_ticks(100_000), 4_096);

                let executed = unsafe { high_ips_emu.run_cpu_batch(100_000) }.unwrap();
                assert!(
                    (1_024..=8_192).contains(&executed),
                    "high-IPS active batch should still yield before BIOS poll loops can time out, got {executed}"
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
    fn hlt_wait_step_uses_millisecond_quantum_without_near_pc_timer() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();

                assert_eq!(emu.hlt_wait_step_ticks(), 4_000);
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

    #[test]
    fn software_reset_requests_preserve_device_state() {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let config = EmulatorConfig::default();
                let mut emu = Emulator::<Corei7SkylakeX>::new(config).unwrap();
                emu.reset(ResetReason::Hardware).unwrap();
                emu.device_manager.pic.master.imr = 0x00;

                assert!(emu.write_port_92h(0x03));
                assert!(emu.check_and_handle_resets().unwrap());

                assert_eq!(
                    emu.device_manager.pic.master.imr, 0x00,
                    "Bochs software reset must not reset devices"
                );
                assert!(emu.device_manager.port92.reset_request.is_none());
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
                    cpu.cpuid(&instr);
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
                    cpu.cpuid(&instr);
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
                    cpu.cpuid(&instr);
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
                    cpu.cpuid(&instr);
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
                emu.memory
                    .load_RAM(
                        &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN],
                        AP_TRAMPOLINE_ADDR,
                    )
                    .unwrap();

                emu.cpu.activity_state = CpuActivityState::WaitForSipi;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);
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
                emu.memory
                    .load_RAM(
                        &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN],
                        AP_TRAMPOLINE_ADDR,
                    )
                    .unwrap();

                let before = emu.cpu_ref(AP_INDEX).icount;

                {
                    let bsp = emu.cpu_mut_at(BSP_INDEX);
                    bsp.lapic.write_aligned(ICR_HIGH, TARGET_APIC_ID << 24);
                    bsp.lapic.write_aligned(
                        ICR_LOW,
                        ((crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8)
                            | ICR_LEVEL_ASSERT
                            | ICR_TRIGGER_LEVEL,
                    );
                    bsp.lapic.write_aligned(
                        ICR_LOW,
                        (crate::cpu::apic::ApicDeliveryMode::Init as u32) << 8 | ICR_TRIGGER_LEVEL,
                    );
                    bsp.lapic.write_aligned(ICR_HIGH, TARGET_APIC_ID << 24);
                    bsp.lapic.write_aligned(
                        ICR_LOW,
                        AP_TRAMPOLINE_VECTOR as u32
                            | ((crate::cpu::apic::ApicDeliveryMode::Sipi as u32) << 8)
                            | ICR_LEVEL_ASSERT,
                    );
                }

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
                emu.memory
                    .load_RAM(
                        &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN],
                        AP_TRAMPOLINE_ADDR,
                    )
                    .unwrap();
                emu.memory
                    .load_RAM(
                        &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN],
                        SECOND_TRAMPOLINE_ADDR,
                    )
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
                    ap.lapic.write_aligned(0x310, (BSP_INDEX as u32) << 24);
                    ap.lapic.write_aligned(0x300, IN_FLIGHT_VECTOR);
                }
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
                    bsp.lapic.write_aligned(ICR_HIGH, ICR_TARGET_AP << 24);
                    bsp.lapic.write_aligned(
                        ICR_LOW,
                        (crate::cpu::apic::ApicDeliveryMode::Smi as u32) << 8,
                    );
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
                emu.memory
                    .load_RAM(
                        &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN],
                        AP_TRAMPOLINE_ADDR,
                    )
                    .unwrap();

                emu.cpu.activity_state = CpuActivityState::Hlt;
                emu.cpu.pending_event = 0;
                emu.cpu.async_event = 0;
                emu.cpu.lapic.intr = false;
                emu.cpu.lapic.intr_pending = false;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                assert!(
                    !emu.cpu_runnable_for_batch(BSP_INDEX),
                    "test requires a started halted peer with no runnable event"
                );
                assert!(emu.cpu_runnable_for_batch(AP_INDEX));

                let before = emu.cpu_ref(AP_INDEX).icount;
                let elapsed = unsafe { emu.run_cpu_batch(BOCHS_SMP_QUANTUM_TICKS) }.unwrap();
                let ap_delta = emu.cpu_ref(AP_INDEX).icount - before;

                assert!(elapsed >= BOCHS_SMP_QUANTUM_TICKS);
                assert!(
                    elapsed < ap_delta,
                    "elapsed ticks {elapsed} were not averaged with the halted peer quantum; AP delta was {ap_delta}"
                );
                assert_eq!(
                    emu.smp_tick_remainder,
                    (BOCHS_SMP_QUANTUM_TICKS + ap_delta) % 2
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn smp_batch_exposes_elapsed_peer_quantum_to_guest_time_reads() {
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
                emu.memory
                    .load_RAM(&[0x0F, 0x31, 0xF4], AP_TRAMPOLINE_ADDR)
                    .unwrap();

                emu.cpu.activity_state = CpuActivityState::Hlt;
                emu.cpu.pending_event = 0;
                emu.cpu.async_event = 0;
                emu.cpu.lapic.intr = false;
                emu.cpu.lapic.intr_pending = false;
                emu.cpu_mut_at(AP_INDEX).deliver_sipi(AP_TRAMPOLINE_VECTOR);

                let _elapsed = unsafe { emu.run_cpu_batch(BOCHS_SMP_QUANTUM_TICKS) }.unwrap();
                assert!(
                    emu.cpu_ref(AP_INDEX).rax() > 0,
                    "AP RDTSC did not observe halted peer quantum elapsed earlier in the SMP round"
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
                emu.memory
                    .load_RAM(
                        &[AP_TRAMPOLINE_OPCODE; AP_TRAMPOLINE_LEN],
                        AP_TRAMPOLINE_ADDR,
                    )
                    .unwrap();

                let handle = emu
                    .pc_system
                    .register_timer(TimerOwner::Lapic(AP_INDEX), 1, false, false, "ap_lapic")
                    .unwrap();

                emu.cpu.activity_state = CpuActivityState::Hlt;
                emu.cpu.pending_event = 0;
                emu.cpu.async_event = 0;
                emu.cpu.lapic.intr = false;
                emu.cpu.lapic.intr_pending = false;

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.lapic.timer_handle = Some(handle);
                    ap.lapic.write_aligned(0xF0, 0x1FF);
                    ap.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR);
                    ap.lapic.set_initial_timer_count(1);
                    ap.deliver_sipi(AP_TRAMPOLINE_VECTOR);
                }
                emu.service_lapic_local_events();

                let elapsed = unsafe { emu.run_cpu_batch(BOCHS_SMP_QUANTUM_TICKS) }.unwrap();

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

                assert!(
                    !emu.can_fast_forward_bsp_hlt(),
                    "once an AP is active, the SMP scheduler must own HLT progress"
                );
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
                    cpu.lapic.write_aligned(0xF0, 0x1FF);
                    if cpu_index != BSP_INDEX {
                        cpu.activity_state = CpuActivityState::Hlt;
                        cpu.set_rflags_for_api(0x202);
                        cpu.pending_event = 0;
                        cpu.async_event = 0;
                        cpu.lapic.intr = false;
                        cpu.lapic.intr_pending = false;
                    }
                }

                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x300, ICR_ALL_BUT_SELF | FIXED_IPI_VECTOR);
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
                emu.memory.load_RAM(&ivt_entry, 8).unwrap();
                emu.memory.load_RAM(&[0xF4], NMI_HANDLER_ADDR).unwrap();

                for cpu_index in 0..emu.cpu_count() {
                    emu.cpu_mut_at(cpu_index).lapic.write_aligned(0xF0, 0x1FF);
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
                emu.cpu.activity_state = CpuActivityState::Hlt;
                emu.cpu.pending_event = 0;
                emu.cpu.async_event = 0;

                assert!(
                    !emu.cpu_runnable_for_batch(AP_INDEX),
                    "shutdown AP with no pending event must stay unscheduled"
                );
                assert!(
                    emu.can_fast_forward_bsp_hlt(),
                    "idle shutdown AP must not disable the BSP HLT pacing path"
                );

                // BSP sends a physical-destination NMI IPI to the AP.
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x310, (AP_INDEX as u32) << 24);
                emu.cpu_mut_at(BSP_INDEX)
                    .lapic
                    .write_aligned(0x300, ICR_DELIVERY_NMI);
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
                emu.cpu.activity_state = CpuActivityState::Hlt;

                {
                    let ap = emu.cpu_mut_at(AP_INDEX);
                    ap.activity_state = CpuActivityState::Hlt;
                    ap.set_rflags_for_api(0x202);
                    ap.lapic.write_aligned(0xF0, 0x1FF);
                    ap.lapic.write_aligned(0x320, 0x30);
                    ap.lapic.set_initial_timer_count(1);
                    ap.lapic.timer_fired = true;
                }

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
                    ap.lapic.write_aligned(0xF0, 0x1FF);
                    ap.lapic
                        .write_aligned(0x320, LVT_TIMER_PERIODIC_MODE | TEST_LAPIC_TIMER_VECTOR);
                    ap.lapic
                        .set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32);
                }

                emu.service_lapic_local_events();
                emu.pc_system.tickn(TEST_LAPIC_TIMER_ELAPSED_TICKS);
                emu.dispatch_timer_fires();
                emu.service_lapic_local_events();

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
                    ap.lapic.write_aligned(0xF0, 0x1FF);
                    ap.lapic
                        .write_aligned(0x320, LVT_TIMER_PERIODIC_MODE | TEST_LAPIC_TIMER_VECTOR);
                    ap.lapic
                        .set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32);
                }

                emu.service_lapic_local_events();
                emu.pc_system.tickn(ELAPSED_TICKS);
                emu.dispatch_timer_fires();
                emu.service_lapic_local_events();

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
                    bsp.lapic.write_aligned(0xF0, 0x1FF);
                    bsp.lapic.write_aligned(0x320, TEST_LAPIC_TIMER_VECTOR);
                    bsp.lapic
                        .set_initial_timer_count(TEST_LAPIC_TIMER_PERIOD_TICKS as u32);
                }

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
                    lapic.write_aligned(0xF0, 0x1FF);
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
                let expected =
                    (retired + BOCHS_SMP_QUANTUM_TICKS * (cpu_count - 1)) / cpu_count;
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
}
