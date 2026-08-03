#![allow(unused_assignments, dead_code)]

mod error;
pub(crate) mod memory_rusty_box;
pub mod memory_stub;
pub mod misc_mem;
pub mod mmio;
pub mod permissions;

#[cfg(test)]
mod tests;

pub use super::error::Result;
use crate::{
    config::{BxPhyAddress, MAX_HANDLER_OVERFLOW, MAX_MEM_BLOCKS},
    cpu::{BxCpuC, BxCpuIdTrait},
};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
pub use error::*;

use core::cell::{Cell, UnsafeCell};

/// The fixed TLB host-pointer capacities are architectural CPU cache sizes.
///
/// They deliberately live with the descriptor rather than the CPU: eviction
/// checks may run while a CPU is mutably borrowed for instruction execution.
/// These mirror `BX_DTLB_SIZE` / `BX_ITLB_SIZE` (cpu.rs, Bochs cpu.h) and must
/// stay `>=` them — each pin array is indexed by TLB slot, so under-sizing
/// would let a slot index run past the end.
pub(crate) const CPU_TLB_PIN_DTLB_SLOTS: usize = 2048;
pub(crate) const CPU_TLB_PIN_ITLB_SLOTS: usize = 1024;

/// Pin-visible host pointers copied out of one CPU.
///
/// The emulator is single-threaded. It refreshes this state before wiring a
/// CPU memory scope and each CPU synchronously updates it after changing a
/// TLB/VMCB host pointer or invalidating a TLB entry. `UnsafeCell` permits
/// those updates through a stable descriptor without touching the mutably
/// borrowed CPU during an allocator eviction check.
struct CpuTlbPinState {
    dtlb_hosts: [usize; CPU_TLB_PIN_DTLB_SLOTS],
    itlb_hosts: [usize; CPU_TLB_PIN_ITLB_SLOTS],
    vmcb_host: usize,
    /// Bounded non-ITLB instruction-fetch window (`eip_fetch_ptr`) host
    /// interval; `fetch_window_end == 0` means no window. Bochs cpu.cc
    /// `prefetch`: `eipFetchPtr` stays valid until the next refill, so its
    /// backing block must never be evicted while retained.
    fetch_window_start: usize,
    fetch_window_end: usize,
}

impl CpuTlbPinState {
    const fn empty() -> Self {
        Self {
            dtlb_hosts: [0; CPU_TLB_PIN_DTLB_SLOTS],
            itlb_hosts: [0; CPU_TLB_PIN_ITLB_SLOTS],
            vmcb_host: 0,
            fetch_window_start: 0,
            fetch_window_end: 0,
        }
    }
}

/// A stable, external view of one CPU's direct host-memory references.
///
/// This descriptor never retains a CPU pointer. Its interior state is updated
/// only by the owning CPU while the emulator has exclusive machine access, and
/// it remains separately addressable while that CPU has an active `&mut`
/// borrow. Allocator checks therefore inspect only this sidecar.
pub(crate) struct CpuTlbPin {
    state: UnsafeCell<CpuTlbPinState>,
}

impl CpuTlbPin {
    pub(crate) fn new<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
        cpu: &BxCpuC<'_, I, T>,
    ) -> Self {
        let pin = Self {
            state: UnsafeCell::new(CpuTlbPinState::empty()),
        };
        cpu.refresh_tlb_pin(&pin);
        pin
    }

    #[inline]
    pub(crate) fn clear_tlb_hosts(&self) {
        // SAFETY: sidecar mutation is serialized by the emulator's
        // single-threaded CPU/memory scope contract.
        let state = unsafe { &mut *self.state.get() };
        state.dtlb_hosts.fill(0);
        state.itlb_hosts.fill(0);
        state.vmcb_host = 0;
        state.fetch_window_start = 0;
        state.fetch_window_end = 0;
    }

    #[inline]
    pub(crate) fn set_dtlb_host(&self, slot: usize, host: usize) {
        debug_assert!(slot < CPU_TLB_PIN_DTLB_SLOTS);
        // SAFETY: see `clear_tlb_hosts`.
        unsafe { (*self.state.get()).dtlb_hosts[slot] = host };
    }

    #[inline]
    pub(crate) fn set_itlb_host(&self, slot: usize, host: usize) {
        debug_assert!(slot < CPU_TLB_PIN_ITLB_SLOTS);
        // SAFETY: see `clear_tlb_hosts`.
        unsafe { (*self.state.get()).itlb_hosts[slot] = host };
    }

    #[inline]
    pub(crate) fn set_vmcb_host(&self, host: usize) {
        // SAFETY: see `clear_tlb_hosts`.
        unsafe { (*self.state.get()).vmcb_host = host };
    }

    /// Publish (or clear, with `len == 0`) the bounded instruction-fetch
    /// window so eviction never steals the block backing `eip_fetch_ptr`.
    #[inline]
    pub(crate) fn set_fetch_window(&self, start: usize, len: usize) {
        // SAFETY: see `clear_tlb_hosts`.
        let state = unsafe { &mut *self.state.get() };
        state.fetch_window_start = start;
        state.fetch_window_end = start.wrapping_add(len);
    }

    #[inline]
    pub(crate) fn is_range_pinned(&self, start: usize, end: usize) -> bool {
        // SAFETY: eviction checks only read this separately addressable state;
        // mutation is serialized before/after each CPU instruction operation.
        let state = unsafe { &*self.state.get() };
        let contains = |host: usize| host != 0 && host >= start && host < end;
        contains(state.vmcb_host)
            || (state.fetch_window_start < end && state.fetch_window_end > start)
            || state.dtlb_hosts.iter().copied().any(contains)
            || state.itlb_hosts.iter().copied().any(contains)
    }

    /// Exact equality of every published host pin. Used by the Track B property
    /// test to assert that incrementally maintained sidecars stay byte-identical
    /// to a fresh `refresh_tlb_pin` rescan after each TLB operation.
    #[cfg(test)]
    pub(crate) fn state_matches(&self, other: &CpuTlbPin) -> bool {
        // SAFETY: single-threaded test access; no concurrent sidecar mutation.
        let a = unsafe { &*self.state.get() };
        let b = unsafe { &*other.state.get() };
        a.dtlb_hosts == b.dtlb_hosts
            && a.itlb_hosts == b.itlb_hosts
            && a.vmcb_host == b.vmcb_host
            && a.fetch_window_start == b.fetch_window_start
            && a.fetch_window_end == b.fetch_window_end
    }
}
/// The only CPU state consumed by handler-aware physical-memory operations.
///
/// It is computed while the CPU is ordinarily reborrowed, before memory is
/// mutably borrowed.  Memory must never need a shared `BxCpuC` reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CpuMemoryPolicy {
    smm_mode: bool,
    monitor_hit: bool,
    /// Whether this access comes from CPU context. Bochs memory.cc wraps the
    /// whole SMRAM window in `if (cpu != NULL) { ... }`, so a device access
    /// (DMA, an MMIO device writing memory) never reaches SMRAM through it
    /// and falls through to the normal handler/VGA routing instead.
    cpu_context: bool,
}

/// CPU context is the default: every production caller except the device
/// paths below computes its policy from live CPU state, and defaulting the
/// other way would silently hide SMRAM from the CPU.
impl Default for CpuMemoryPolicy {
    fn default() -> Self {
        Self {
            smm_mode: false,
            monitor_hit: false,
            cpu_context: true,
        }
    }
}

impl CpuMemoryPolicy {
    #[inline]
    pub(crate) const fn new(smm_mode: bool, monitor_hit: bool) -> Self {
        Self {
            smm_mode,
            monitor_hit,
            cpu_context: true,
        }
    }

    /// Policy for an access issued by a device rather than a CPU — Bochs's
    /// `cpu == NULL`. Such accesses never see the SMRAM window.
    #[inline]
    pub(crate) const fn device() -> Self {
        Self {
            smm_mode: false,
            monitor_hit: false,
            cpu_context: false,
        }
    }

    /// Bochs memory.cc `cpu != NULL` — gates the SMRAM shortcut.
    #[inline]
    pub(crate) const fn is_cpu_context(self) -> bool {
        self.cpu_context
    }

    #[inline]
    pub(crate) const fn smm_mode(self) -> bool {
        self.smm_mode
    }

    #[inline]
    pub(crate) const fn monitor_hit(self) -> bool {
        self.monitor_hit
    }
}

#[cfg(feature = "std")]
use std::fs::File;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Block {
    Block { offset: usize },
    SwappedOut,
}

/// Block-logical RAM metadata saved by the snapshot layer.
///
/// The backing store deliberately exposes no guest-wide byte slice: an
/// undersized host allocation can keep arbitrary guest blocks swapped out.
#[cfg(feature = "std")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MemorySnapshotGeometry {
    pub guest_len: u64,
    pub host_ram_len: u64,
    pub block_size: u64,
    pub num_blocks: u32,
    pub resident_capacity: u32,
    pub used_blocks: u32,
    pub next_swapout_guest_block: u32,
}

/// Where a logical guest block resides at the snapshot boundary.
#[cfg(feature = "std")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemorySnapshotResidency {
    Swapped,
    Resident { slot: u32 },
}
#[derive(Debug)]
pub struct BxMemoryStubC {
    /// could be > 4G
    pub(super) len: usize,
    /// could be > 4G
    allocated: usize,
    /// Complete block slots available for resident guest RAM. This may exceed
    /// `allocated` by less than one block when swapped backing is active.
    resident_backing_len: usize,
    /// individual block size, must be power of 2
    block_size: usize,
    actual_vector: *mut u8,
    actual_vector_len: usize,
    /// Allocation layout for owned buffers; `None` for external raw memory.
    actual_vector_layout: Option<core::alloc::Layout>,
    /// aligned correctly
    vector_offset: usize,
    /// None if swapped out
    blocks_offsets: UnsafeCell<[Block; MAX_MEM_BLOCKS]>,
    num_blocks: usize,
    /// 512k BIOS rom space + 128k expansion rom space
    rom_offset: usize,
    /// 4k for unexisting memory
    bogus_offset: usize,

    used_blocks: Cell<usize>,

    /// Machine-wide SMC page-write-stamp table — Bochs icache.h
    /// `bxPageWriteStampTable::fineGranularityMapping`. ONE table for the
    /// whole machine (Bochs has a single global instance): trace creation by
    /// ANY cpu marks it, and a write hitting marked lines must invalidate
    /// EVERY cpu's icache (Bochs icache.cc `handleSMC` loops over
    /// BX_SMP_PROCESSORS).
    #[cfg(feature = "alloc")]
    smc_stamps: Vec<u32>,
    #[cfg(not(feature = "alloc"))]
    smc_stamps: [u32; crate::cpu::icache::SMC_STAMP_ENTRIES],
    /// Queued cross-cpu SMC invalidations, drained by the emulator at
    /// round-robin slice boundaries (no sibling cpu can execute before the
    /// drain, so deferral is observably identical to Bochs's synchronous
    /// `handleSMC` loop). `smc_seq_next` is a monotonic event counter; each
    /// cpu keeps a watermark (`BxCpuC::smc_seq_seen`) so it applies exactly
    /// the events it has not seen.
    smc_pending: [crate::cpu::icache::PendingSmc; crate::cpu::icache::SMC_PENDING_CAP],
    smc_pending_len: usize,
    smc_seq_next: u64,
    /// CPUs whose watermark is below this must flush their whole icache
    /// (an event was dropped on pending-queue overflow).
    smc_overflow_seq: u64,

    /// Zero-initialized 4KB scratch buffer for APIC MMIO (0xFEE00000-0xFEEFFFFF)
    apic_scratch: [u8; 4096],

    next_swapout_idx: Cell<usize>,

    /// Cached "host backing is a full identity map" verdict consumed by
    /// `identity_guest_base` on every cpu-loop entry (per SMP slice — the
    /// O(num_blocks) table walk this replaces dominated the SMP hot path).
    /// Maintained at every block-table mutation: construction, block
    /// allocation/eviction, and snapshot restore.
    identity_map: Cell<bool>,
    #[cfg(feature = "std")]
    //overflow_file: Option<Arc<Mutex<std::fs::File>>>,
    overflow_file: UnsafeCell<File>,
    //swapped_out: *const u8,
}

// SAFETY: The raw pointer `actual_vector` is owned exclusively by this struct
// (allocated once, never aliased). UnsafeCell fields are only accessed single-threaded.
unsafe impl Send for BxMemoryStubC {}

impl Drop for BxMemoryStubC {
    fn drop(&mut self) {
        #[cfg(feature = "alloc")]
        if let Some(layout) = self.actual_vector_layout.take() {
            unsafe { alloc::alloc::dealloc(self.actual_vector, layout) };
        }
    }
}

type Unsigned = u32;

/// Identifies which device owns a memory-mapped I/O handler.
///
/// Each variant carries a raw pointer to the device instance, replacing the
/// former `*const c_void` param + fn-ptr pair with a typed discriminant that
/// the dispatch code in `misc_mem.rs` matches on directly.
#[derive(Clone, Copy)]
pub(crate) enum MemoryDeviceId {
    Vga(*mut crate::iodev::vga::BxVgaC),
    IoApic(*mut crate::iodev::ioapic::BxIoApic),
    Hpet(*mut crate::iodev::hpet::BxHpetC),
    None,
}

impl core::fmt::Debug for MemoryDeviceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Vga(p) => write!(f, "Vga({:p})", p),
            Self::IoApic(p) => write!(f, "IoApic({:p})", p),
            Self::Hpet(p) => write!(f, "Hpet({:p})", p),
            Self::None => write!(f, "None"),
        }
    }
}

impl MemoryDeviceId {
    /// Dereference the VGA device pointer.
    ///
    /// # Safety (internal)
    /// The raw pointer was set once at init and remains valid for the emulator lifetime.
    /// Aliasing is the caller's responsibility (same as the prior inline `unsafe` blocks).
    #[inline(always)]
    pub(crate) fn vga_mut(&self) -> Option<&mut crate::iodev::vga::BxVgaC> {
        match self {
            MemoryDeviceId::Vga(ptr) => Some(unsafe { &mut **ptr }),
            _ => None,
        }
    }

    /// Dereference the IOAPIC device pointer.
    ///
    /// # Safety (internal)
    /// The raw pointer was set once at init and remains valid for the emulator lifetime.
    /// Aliasing is the caller's responsibility (same as the prior inline `unsafe` blocks).
    #[inline(always)]
    pub(crate) fn ioapic_mut(&self) -> Option<&mut crate::iodev::ioapic::BxIoApic> {
        match self {
            MemoryDeviceId::IoApic(ptr) => Some(unsafe { &mut **ptr }),
            _ => None,
        }
    }

    /// Dereference the HPET device pointer.
    ///
    /// # Safety (internal)
    /// The raw pointer was set once at init and remains valid for the emulator lifetime.
    /// Aliasing is the caller's responsibility (same as the prior inline `unsafe` blocks).
    #[inline(always)]
    pub(crate) fn hpet_mut(&self) -> Option<&mut crate::iodev::hpet::BxHpetC> {
        match self {
            MemoryDeviceId::Hpet(ptr) => Some(unsafe { &mut **ptr }),
            _ => None,
        }
    }

    /// Whether two ids refer to the same device instance (pointer identity).
    /// Used by `unregister_memory_handlers` to match the handler to remove.
    #[inline]
    pub(crate) fn same_device(&self, other: &MemoryDeviceId) -> bool {
        match (self, other) {
            (MemoryDeviceId::Vga(a), MemoryDeviceId::Vga(b)) => core::ptr::eq(*a, *b),
            (MemoryDeviceId::IoApic(a), MemoryDeviceId::IoApic(b)) => core::ptr::eq(*a, *b),
            (MemoryDeviceId::Hpet(a), MemoryDeviceId::Hpet(b)) => core::ptr::eq(*a, *b),
            (MemoryDeviceId::None, MemoryDeviceId::None) => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
pub(super) struct MemoryHandlerStruct {
    next: Option<u16>,
    pub(super) begin: BxPhyAddress,
    pub(super) end: BxPhyAddress,
    bitmap: u16,
    pub(super) device_id: MemoryDeviceId,
}

//#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)

pub(crate) const BIOS_ROM_LOWER: u8 = 0x01;
pub(crate) const BIOS_ROM_EXTENDED: u8 = 0x02;
pub(crate) const BIOS_ROM_1MEG: u8 = 0x04;

#[derive(Debug)]
pub struct BxMemC<'a> {
    #[cfg(feature = "alloc")]
    memory_handlers: Vec<Option<MemoryHandlerStruct>>,
    #[cfg(not(feature = "alloc"))]
    memory_handlers: [Option<MemoryHandlerStruct>; 4096],
    handler_overflow: [Option<MemoryHandlerStruct>; MAX_HANDLER_OVERFLOW],
    handler_overflow_count: usize,
    pci_enabled: bool,
    bios_write_enabled: bool,

    smram_available: bool,
    smram_enable: bool,
    smram_restricted: bool,

    rom_present: [bool; 65],
    memory_type: [[bool; 2]; 13],
    bios_rom_addr: u32,
    bios_rom_access: u8,
    flash_type: u8,
    flash_status: u8,
    flash_wsm_state: u8,
    flash_modified: bool,

    inherited_memory_stub: BxMemoryStubC,

    /// A20 address mask - controls address line 20 gating
    /// This is synchronized from BxPcSystemC when A20 state changes
    a20_mask: BxPhyAddress,

    /// `(system_ticks, ips)` of the in-flight HPET MMIO access, stamped by
    /// the CPU slow path before dispatch. The HPET converts this to the
    /// nanosecond clock Bochs reads via `bx_pc_system.time_nsec()` inside
    /// its handlers; a plain field would need `&mut` on read paths.
    hpet_access_clock: core::cell::Cell<(u64, u64)>,

    /// Keeps the lifetime parameter used by callers (CPU borrows, emulator context).
    _marker: core::marker::PhantomData<&'a ()>,
}

impl BxMemC<'_> {
    /// Apply A20 masking to an address
    #[inline]
    pub fn a20_addr(&self, addr: BxPhyAddress) -> BxPhyAddress {
        addr & self.a20_mask
    }

    /// Set the A20 mask (called when A20 line state changes)
    pub fn set_a20_mask(&mut self, mask: BxPhyAddress) {
        self.a20_mask = mask;
    }


    /// Get the current A20 mask
    pub fn a20_mask(&self) -> BxPhyAddress {
        self.a20_mask
    }

    /// Stamp the emulated clock for an in-flight HPET MMIO access — the CPU
    /// slow path records its `system_ticks()`/`ips` pair here so the HPET
    /// handler observes the same clock Bochs reads via
    /// `bx_pc_system.time_nsec()` inside `hpet_read`/`hpet_write`.
    #[inline]
    pub(crate) fn stamp_hpet_access_clock(&self, system_ticks: u64, ips: u64) {
        self.hpet_access_clock.set((system_ticks, ips));
    }

    /// The `(system_ticks, ips)` pair stamped for the current HPET access.
    #[inline]
    pub(crate) fn hpet_access_clock(&self) -> (u64, u64) {
        self.hpet_access_clock.get()
    }


    // ── SMC write-stamp table forwarders (table lives in the stub) ─────────

    /// Bochs icache.h `bxPageWriteStampTable::markICacheMask`.
    #[inline]
    pub(crate) fn smc_mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        self.inherited_memory_stub
            .smc_mark_icache_mask(p_addr, mask);
    }

    /// Whether a single-page physical range overlaps cached instruction lines.
    #[inline]
    pub(crate) fn smc_range_has_stamps(&self, p_addr: BxPhyAddress, len: u32) -> bool {
        self.inherited_memory_stub.smc_range_has_stamps(p_addr, len)
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp(pAddr, len)`.
    #[inline]
    pub(crate) fn smc_dec_write_stamp(&mut self, p_addr: BxPhyAddress, len: u32) {
        self.inherited_memory_stub.smc_dec_write_stamp(p_addr, len);
    }

    /// Sequence number the next SMC event will get (cpu watermark compare).
    #[inline]
    pub(crate) fn smc_seq_next(&self) -> u64 {
        self.inherited_memory_stub.smc_seq_next()
    }

    /// Events a watermark of `since` has not seen: `(needs_full_flush, events)`.
    #[inline]
    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        self.inherited_memory_stub.smc_pending_since(since)
    }

    /// Drop drained SMC events (emulator, after every cpu caught up).
    #[inline]
    pub(crate) fn smc_clear_pending(&mut self) {
        self.inherited_memory_stub.smc_clear_pending();
    }

    /// True when SMC events are queued (per-slice drain early-out).
    #[inline]
    pub(crate) fn smc_has_pending(&self) -> bool {
        self.inherited_memory_stub.smc_has_pending()
    }

    /// Bochs icache.h `bxPageWriteStampTable::resetWriteStamps` (hardware reset).
    pub(crate) fn smc_reset_stamps(&mut self) {
        self.inherited_memory_stub.smc_reset_stamps();
    }

    /// Enable SMRAM (System Management RAM) with the given DOPEN/DCLS state.
    ///
    /// Matches BX_MEM_C::enable_smram(bool enable, bool restricted) from
    /// cpp_orig/bochs/memory/misc_mem.cc: `enable` is DOPEN (SMM space open
    /// for non-SMM-mode CPU accesses), `restricted` is DCLS (SMM space closed
    /// to data references while still open to code fetches).
    pub fn enable_smram(&mut self, enable: bool, restricted: bool) {
        self.smram_available = true;
        self.smram_enable = enable;
        self.smram_restricted = restricted;
    }

    /// Disable SMRAM (System Management RAM)
    ///
    /// Matches BX_MEM_C::disable_smram() from cpp_orig/bochs/memory/misc_mem.cc
    pub fn disable_smram(&mut self) {
        self.smram_available = false;
        self.smram_enable = false;
        self.smram_restricted = false;
    }

    /// Snapshot of SMRAM control state: (available, enable/DOPEN, restricted/DCLS).
    /// Test/diagnostic accessor — the actual A0000-BFFFF routing decision is
    /// made directly against these flags in misc_mem.rs's pin-aware host
    /// mapping and physical read/write paths, untouched here.
    pub(crate) fn smram_state(&self) -> (bool, bool, bool) {
        (
            self.smram_available,
            self.smram_enable,
            self.smram_restricted,
        )
    }

    /// Test/diagnostic accessor for the PAM-derived memory type of a shadow
    /// RAM area. Mirrors `set_memory_type`: `area` is one of the 13 memory
    /// areas (C0000..F0000), `rw` is 0 = read path, 1 = write path.
    pub(crate) fn memory_type(&self, area: usize, rw: usize) -> bool {
        self.memory_type[area][rw]
    }

    /// Set whether writes to the BIOS ROM region are allowed outside PAM
    /// shadow RAM (both the C0000-FFFFF non-shadowed path and the
    /// top-of-address-space BIOS mirror). Driven by the PIIX3 XBCS register
    /// bit 2 (Bochs pci2isa.cc `pci_write_handler` case 0x4e ->
    /// `DEV_mem_set_bios_write`); Bochs `BX_MEM_C::set_bios_write`
    /// (misc_mem.cc).
    pub fn set_bios_write_enabled(&mut self, enabled: bool) {
        self.bios_write_enabled = enabled;
    }

    /// Test/diagnostic accessor for the current BIOS-write-enable state.
    pub(crate) fn bios_write_enabled(&self) -> bool {
        self.bios_write_enabled
    }

    /// Set or clear one region bit of the BIOS ROM access bitmask (`region`
    /// is one of `BIOS_ROM_LOWER`/`BIOS_ROM_EXTENDED`/`BIOS_ROM_1MEG`).
    /// Matches Bochs `BX_MEM_C::set_bios_rom_access` (misc_mem.cc), driven by
    /// PIIX3 XBCS bits 6-7 (pci2isa.cc case 0x4e ->
    /// `DEV_mem_set_bios_rom_access`). As in upstream Bochs, this bitmask is
    /// tracked for parity but not consulted by any read/write path — Bochs
    /// itself logs "BIOS enable switches not supported" when these bits
    /// change and never reads `bios_rom_access` back anywhere.
    pub fn set_bios_rom_access(&mut self, region: u8, enabled: bool) {
        if enabled {
            self.bios_rom_access |= region;
        } else {
            self.bios_rom_access &= !region;
        }
    }

    /// Test/diagnostic accessor for the BIOS ROM access bitmask.
    pub(crate) fn bios_rom_access(&self) -> u8 {
        self.bios_rom_access
    }
}

// implement getters and setters for memory stub
impl BxMemoryStubC {
    /// Reconstruct the complete owned backing slice for memory-internal
    /// storage operations only. It is never a guest-linear RAM view.
    pub(super) fn actual_vector_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.actual_vector, self.actual_vector_len) }
    }

    /// Mutable counterpart to `actual_vector_slice`, restricted to memory
    /// internals such as ROM and resident-slot maintenance.
    pub(super) fn actual_vector_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.actual_vector, self.actual_vector_len) }
    }

    #[allow(clippy::mut_from_ref)]
    fn blocks_offsets(&self) -> &mut [Block] {
        let arr = unsafe { &mut (*self.blocks_offsets.get()) };
        &mut arr[..self.num_blocks]
    }

    /// Full O(num_blocks) identity-map scan — the ground truth behind the
    /// cached `identity_map` flag. Used to (re)compute the cache at block
    /// table rewrites and as the debug oracle in `identity_guest_base`.
    pub(super) fn scan_identity_map(&self) -> bool {
        self.allocated >= self.len
            && self
                .blocks_offsets()
                .iter()
                .enumerate()
                .all(|(guest_block, block)| {
                    matches!(
                        block,
                        Block::Block { offset } if *offset == guest_block * self.block_size
                    )
                })
    }

    /// Re-derive the cached identity verdict after a bulk block-table rewrite.
    pub(super) fn recompute_identity_map(&self) {
        self.identity_map.set(self.scan_identity_map());
    }

    pub(super) fn rom(&mut self) -> &mut [u8] {
        let ro = self.rom_offset;
        &mut self.actual_vector_mut()[ro..]
    }

    pub(super) fn bogus(&mut self) -> &mut [u8] {
        let bo = self.bogus_offset;
        &mut self.actual_vector_mut()[bo..]
    }

    pub(super) fn apic_scratch(&mut self) -> &mut [u8] {
        &mut self.apic_scratch
    }

    #[allow(clippy::mut_from_ref)]
    pub(super) fn block_by_index(&self, index: usize) -> Option<&mut [u8]> {
        if let Some(Block::Block { offset }) = self.blocks_offsets().get(index) {
            let start = self.vector_offset + *offset;
            // SAFETY: We're accessing within bounds of actual_vector via interior mutability pattern
            let slice = unsafe {
                core::slice::from_raw_parts_mut(self.actual_vector.add(start), self.block_size)
            };
            Some(slice)
        } else {
            None
        }
    }

    #[cfg(feature = "std")]
    #[allow(clippy::mut_from_ref)]
    fn overflow_file_mut(&self) -> &mut File {
        unsafe { &mut *self.overflow_file.get() }
    }
}
impl<'m> BxMemC<'m> {

    /// Copy RAM through the checked, block-resident backing store. This bypasses
    /// device handlers just like Bochs's physical DMA RAM path.
    pub(crate) fn read_ram(
        &mut self,
        pins: &[CpuTlbPin],
        addr: BxPhyAddress,
        out: &mut [u8],
    ) -> Result<usize> {
        let mut copied = 0usize;
        while copied < out.len() {
            let logical = addr
                .checked_add(u64::try_from(copied)?)
                .ok_or(MemoryError::Internal("RAM read address overflow"))?;
            let a20 = self.a20_addr(logical);
            if memory_rusty_box::bx_is_pci_hole_addr(a20) {
                break;
            }
            let Some(span) = memory_rusty_box::bx_guest_ram_span(
                a20,
                1,
                self.inherited_memory_stub.len,
            ) else {
                break;
            };
            let page_left = 0x1000usize - ((a20 as usize) & 0xfff);
            let hole_left = if a20 < memory_rusty_box::BX_PCI_HOLE_START {
                usize::try_from(memory_rusty_box::BX_PCI_HOLE_START - a20).unwrap_or(usize::MAX)
            } else {
                usize::MAX
            };
            let chunk = {
                let vector = match self.inherited_memory_stub.get_vector_offset(span.start, pins) {
                    Ok(vector) => vector,
                    Err(_) if copied != 0 => return Ok(copied),
                    Err(error) => return Err(error),
                };
                let count = vector
                    .len()
                    .min(page_left)
                    .min(hole_left)
                    .min(out.len() - copied);
                out[copied..copied + count].copy_from_slice(&vector[..count]);
                count
            };
            if chunk == 0 {
                break;
            }
            copied += chunk;
        }
        Ok(copied)
    }

    /// Copy RAM through the checked, block-resident backing store and stamp
    /// precisely the A20-adjusted guest bytes actually committed.
    pub(crate) fn write_ram(
        &mut self,
        pins: &[CpuTlbPin],
        addr: BxPhyAddress,
        data: &[u8],
    ) -> Result<usize> {
        let mut copied = 0usize;
        while copied < data.len() {
            let logical = addr
                .checked_add(u64::try_from(copied)?)
                .ok_or(MemoryError::Internal("RAM write address overflow"))?;
            let a20 = self.a20_addr(logical);
            if memory_rusty_box::bx_is_pci_hole_addr(a20) {
                break;
            }
            let Some(span) = memory_rusty_box::bx_guest_ram_span(
                a20,
                1,
                self.inherited_memory_stub.len,
            ) else {
                break;
            };
            let page_left = 0x1000usize - ((a20 as usize) & 0xfff);
            let hole_left = if a20 < memory_rusty_box::BX_PCI_HOLE_START {
                usize::try_from(memory_rusty_box::BX_PCI_HOLE_START - a20).unwrap_or(usize::MAX)
            } else {
                usize::MAX
            };
            let chunk = {
                let vector = match self.inherited_memory_stub.get_vector_offset(span.start, pins) {
                    Ok(vector) => vector,
                    Err(_) if copied != 0 => return Ok(copied),
                    Err(error) => return Err(error),
                };
                let count = vector
                    .len()
                    .min(page_left)
                    .min(hole_left)
                    .min(data.len() - copied);
                vector[..count].copy_from_slice(&data[copied..copied + count]);
                count
            };
            if chunk == 0 {
                break;
            }
            self.smc_dec_write_stamp(a20, u32::try_from(chunk)?);
            copied += chunk;
        }
        Ok(copied)
    }

    /// Debugger physical write through the block-aware RAM path.
    ///
    /// Debugger writes are all-or-nothing from the debugger's perspective:
    /// a PCI hole, out-of-range address, or short source buffer reports
    /// failure rather than exposing a partial flat backing write.
    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    pub(crate) fn dbg_set_mem(
        &mut self,
        pins: &[CpuTlbPin],
        addr: BxPhyAddress,
        len: u32,
        buf: &[u8],
    ) -> Result<bool> {
        let requested = usize::try_from(len)?;
        if buf.len() < requested {
            return Ok(false);
        }
        Ok(self.write_ram(pins, addr, &buf[..requested])? == requested)
    }

    /// Compute the Bochs debugger CRC32 through fixed-size block-aware reads.
    ///
    /// The first PCI-hole or short/out-of-range read fails the request. This
    /// deliberately never substitutes `0xff` bytes from a flat host backing.
    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    pub(crate) fn dbg_crc32(
        &mut self,
        pins: &[CpuTlbPin],
        addr1: BxPhyAddress,
        addr2: BxPhyAddress,
        crc: &mut u32,
    ) -> Result<bool> {
        let mut c = 0xFFFF_FFFFu32;
        if addr1 > addr2 {
            *crc = c;
            return Ok(true);
        }
        let mut remaining = addr2
            .checked_sub(addr1)
            .and_then(|span| span.checked_add(1))
            .ok_or(MemoryError::Internal("debugger CRC address range overflow"))?;
        let mut addr = addr1;
        let mut scratch = [0u8; 4096];
        while remaining != 0 {
            let chunk = usize::try_from(remaining.min(scratch.len() as u64))?;
            if self.read_ram(pins, addr, &mut scratch[..chunk])? != chunk {
                return Ok(false);
            }
            for &byte in &scratch[..chunk] {
                c ^= u32::from(byte);
                for _ in 0..8 {
                    let mask = 0u32.wrapping_sub(c & 1);
                    c = (c >> 1) ^ (0xEDB8_8320 & mask);
                }
            }
            remaining -= chunk as u64;
            if remaining != 0 {
                addr = addr
                    .checked_add(chunk as u64)
                    .ok_or(MemoryError::Internal("debugger CRC address overflow"))?;
            }
        }
        *crc = c;
        Ok(true)
    }


    pub(crate) fn get_memory_len(&self) -> usize {
        self.inherited_memory_stub.len
    }

    /// Return the guest RAM base only when host backing is a full identity map.
    ///
    /// The null result deliberately prevents consumers from treating swapped
    /// guest RAM as a guest-wide host slice.
    ///
    /// Consumes the cached `identity_map` verdict: this runs on every
    /// cpu-loop entry (per SMP slice), where the previous O(num_blocks)
    /// table walk dominated the whole SMP scheduling path.
    pub(crate) fn identity_guest_base(&mut self) -> (*mut u8, usize) {
        let stub = &self.inherited_memory_stub;
        debug_assert_eq!(
            stub.identity_map.get(),
            stub.scan_identity_map(),
            "cached identity-map verdict diverged from the block table"
        );
        if !stub.identity_map.get() {
            return (core::ptr::null_mut(), 0);
        }
        let ptr = unsafe { stub.actual_vector.add(stub.vector_offset) };
        (ptr, stub.len)
    }

    /// Block-logical snapshot geometry; no caller receives the host backing.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_geometry(&self) -> MemorySnapshotGeometry {
        self.inherited_memory_stub.snapshot_geometry()
    }

    #[cfg(feature = "std")]
    pub(crate) fn snapshot_residency(
        &self,
        guest_block: u32,
    ) -> std::io::Result<MemorySnapshotResidency> {
        self.inherited_memory_stub.snapshot_residency(guest_block)
    }

    #[cfg(feature = "std")]
    pub(crate) fn write_snapshot_block<W: std::io::Write>(
        &self,
        guest_block: u32,
        out: &mut W,
    ) -> std::io::Result<()> {
        self.inherited_memory_stub
            .write_snapshot_block(guest_block, out)
    }

    #[cfg(feature = "std")]
    pub(crate) fn read_snapshot_block<R: std::io::Read>(
        &mut self,
        guest_block: u32,
        saved: MemorySnapshotResidency,
        input: &mut R,
    ) -> std::io::Result<()> {
        self.inherited_memory_stub
            .read_snapshot_block(guest_block, saved, input)
    }

    #[cfg(feature = "std")]
    pub(crate) fn finish_snapshot_restore(
        &mut self,
        geometry: MemorySnapshotGeometry,
        saved_map: &[MemorySnapshotResidency],
    ) -> std::io::Result<()> {
        self.inherited_memory_stub
            .finish_snapshot_restore(geometry, saved_map)
    }

    /// Count how many registered (non-None) memory handlers exist (for diagnostics).
    pub fn memory_handler_info(&self) -> usize {
        self.memory_handlers.iter().filter(|h| h.is_some()).count()
    }

    /// Set memory type for a specific area (PAM register support).
    /// Bochs: BX_MEM_C::set_memory_type() (misc_mem.cc)
    ///
    /// `area` is one of the 13 memory areas (C0000..F0000, 16KB each).
    /// `rw`: 0 = read path, 1 = write path.
    /// `dram`: true = DRAM (shadow RAM), false = ROM.
    pub fn set_memory_type(&mut self, area: usize, rw: usize, dram: bool) {
        if area < 13 && rw < 2 {
            tracing::trace!(
                "set_memory_type: area={}, rw={}, dram={} (was {})",
                area,
                rw,
                dram,
                self.memory_type[area][rw]
            );
            self.memory_type[area][rw] = dram;
        }
    }

    /// Read bytes from the ROM array at the given offset (for diagnostics).
    pub fn peek_rom(&self, offset: usize, len: usize) -> &[u8] {
        let stub = &self.inherited_memory_stub;
        let rom_start = stub.rom_offset;
        let v = stub.actual_vector_slice();
        let rom = &v[rom_start..];
        let end = (offset + len).min(rom.len());
        if offset < rom.len() {
            &rom[offset..end]
        } else {
            &[]
        }
    }
}
#[cfg(feature = "alloc")]
impl<'m> BxMemC<'m> {
    pub fn init_memory(
        &mut self,
        guest_size: usize,
        host_size: usize,
        block_size: usize,
    ) -> Result<()> {
        let mem_stub = BxMemoryStubC::create_and_init(guest_size, host_size, block_size)?;
        self.inherited_memory_stub = *mem_stub;
        self.rom_present = [false; 65];
        self.bios_rom_addr = 0xffff0000;
        self.memory_type = [[false, false]; 13];
        Ok(())
    }
}

#[cfg(all(test, feature = "std"))]
mod phase1_tests {

/// Emulator construction needs more than the default 2 MiB test stack, but
/// far less than the 256 MiB previously reserved here: `Emulator` is ~4 MiB.
/// Oversized reservations across many parallel tests intermittently exhausted
/// the process and failed unrelated tests with STATUS_STACK_OVERFLOW.
const TEST_STACK_SIZE: usize = 64 * MIB;
    use super::{
        memory_rusty_box::*, BxMemC, BxMemoryStubC, CpuMemoryPolicy, CpuTlbPin, MemoryError,
        MemorySnapshotGeometry, MemorySnapshotResidency,
    };
    use std::io::{self, Read, Seek, SeekFrom};
    use crate::{
        cpu::{
            builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX,
            rusty_box::MemoryAccessType,
        },
        Error,
    };

    const MIB: usize = 1024 * 1024;

    fn swapped_memory() -> BxMemC<'static> {
        let mut memory = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, MIB, MIB).expect("memory allocation"),
            false,
        );
        memory.set_a20_mask(u64::MAX);
        memory
    }

    fn snapshot_map(memory: &BxMemC<'_>) -> (MemorySnapshotGeometry, Vec<MemorySnapshotResidency>) {
        let stub = &memory.inherited_memory_stub;
        let geometry = stub.snapshot_geometry();
        let mut map = Vec::with_capacity(geometry.num_blocks as usize);
        for guest_block in 0..geometry.num_blocks {
            map.push(stub.snapshot_residency(guest_block).unwrap());
        }
        (geometry, map)
    }

    struct ShortReader {
        remaining: usize,
    }

    impl Read for ShortReader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let copied = self.remaining.min(out.len());
            if copied != 0 {
                out[..copied].fill(0xa5);
                self.remaining -= copied;
            }
            Ok(copied)
        }
    }

    #[test]
    fn physical_ram_translation_hole_edges() {
        assert_eq!(bx_guest_ram_span(BX_PCI_HOLE_START - 1, 1, 4 * MIB), None);
        assert_eq!(bx_guest_ram_span(BX_PCI_HOLE_START, 0, usize::MAX).unwrap().start as u64, BX_PCI_HOLE_START);
        assert!(bx_guest_ram_span(BX_PCI_HOLE_START, 1, usize::MAX).is_none());
        assert_eq!(
            bx_guest_ram_span(BX_PCI_HOLE_END, 1, 3 * 1024 * 1024 * 1024 + 1)
                .unwrap()
                .start,
            3 * 1024 * 1024 * 1024
        );
        assert!(bx_guest_ram_span((4 * MIB - 1) as u64, 2, 4 * MIB).is_none());
    }

    #[test]
    fn undersized_host_memory_swaps_blocks_without_alias_or_oob() {
        let mut mem = swapped_memory();
        assert_eq!(mem.identity_guest_base(), (core::ptr::null_mut(), 0));
        for block in 0..4usize {
            let value = [0x40 + block as u8];
            assert_eq!(mem.write_ram(&[], (block * MIB) as u64, &value).unwrap(), 1);
        }
        for block in 0..4usize {
            let mut value = [0];
            assert_eq!(mem.read_ram(&[], (block * MIB) as u64, &mut value).unwrap(), 1);
            assert_eq!(value, [0x40 + block as u8], "guest block {block}");
        }
    }

    #[test]
    fn sub_block_host_memory_rounds_up_one_resident_slot() {
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, MIB, 2 * MIB).unwrap(),
            false,
        );
        let (geometry, _) = snapshot_map(&mem);
        assert_eq!(geometry.host_ram_len, MIB as u64);
        assert_eq!(geometry.resident_capacity, 1);
        assert_eq!(geometry.used_blocks, 0);

        assert_eq!(mem.write_ram(&[], 0, &[0x11]).unwrap(), 1);
        assert_eq!(mem.write_ram(&[], (2 * MIB) as u64, &[0x22]).unwrap(), 1);

        let mut first = [0];
        let mut second = [0];
        assert_eq!(mem.read_ram(&[], 0, &mut first).unwrap(), 1);
        assert_eq!(
            mem.read_ram(&[], (2 * MIB) as u64, &mut second).unwrap(),
            1
        );
        assert_eq!((first, second), ([0x11], [0x22]));
    }

    #[test]
    fn undersized_host_first_touch_reads_zero_without_eof() {
        let mut mem = swapped_memory();
        let mut bytes = [0xff; 32];
        assert_eq!(mem.read_ram(&[], (3 * MIB) as u64, &mut bytes).unwrap(), bytes.len());
        assert_eq!(bytes, [0; 32]);
    }

    #[test]
    fn block_aware_ram_copy_crosses_resident_and_swapped_blocks() {
        let mut mem = swapped_memory();
        let source = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(
            mem.write_ram(&[], (MIB - 2) as u64, &source).unwrap(),
            source.len()
        );
        let mut output = [0; 4];
        assert_eq!(
            mem.read_ram(&[], (MIB - 2) as u64, &mut output).unwrap(),
            output.len()
        );
        assert_eq!(output, source);
    }

    #[test]
    fn ram_copy_reports_out_of_range_without_partial_overflow() {
        let mut mem = swapped_memory();
        let data = [0xaa, 0xbb];
        assert_eq!(mem.write_ram(&[], (4 * MIB - 1) as u64, &data).unwrap(), 1);
        let mut last = [0];
        assert_eq!(mem.read_ram(&[], (4 * MIB - 1) as u64, &mut last).unwrap(), 1);
        assert_eq!(last, [0xaa]);
    }

    #[test]
    fn block_aware_ram_copy_reapplies_a20_across_one_megabyte() {
        let mut mem = swapped_memory();
        mem.set_a20_mask(0xFFFF_FFFF_FFEF_FFFF);
        assert_eq!(mem.write_ram(&[], 0x000f_fffe, &[1, 2, 3, 4]).unwrap(), 4);
        let mut low = [0; 2];
        let mut high = [0; 2];
        assert_eq!(mem.read_ram(&[], 0, &mut low).unwrap(), 2);
        assert_eq!(mem.read_ram(&[], 0x000f_fffe, &mut high).unwrap(), 2);
        assert_eq!(low, [3, 4]);
        assert_eq!(high, [1, 2]);
    }

    #[test]
    fn load_ram_above_host_backing_is_complete() {
        let mut mem = swapped_memory();
        let data = [7, 8, 9];
        mem.load_RAM(&[], &data, (3 * MIB) as u64).unwrap();
        let mut output = [0; 3];
        assert_eq!(mem.read_ram(&[], (3 * MIB) as u64, &mut output).unwrap(), 3);
        assert_eq!(output, data);
        assert!(matches!(
            mem.load_RAM(&[], &[1, 2], (4 * MIB - 1) as u64),
            Err(crate::Error::Memory(MemoryError::RamImageOutOfRange))
        ));
    }
    #[test]
    fn snapshot_streams_and_restores_swapped_guest_blocks_without_flat_ram() {
        let mut source = BxMemC::new(
            BxMemoryStubC::create_and_init(5 * MIB, 2 * MIB, 2 * MIB).unwrap(),
            false,
        );
        source.set_a20_mask(u64::MAX);
        source.write_ram(&[], 0, &[0x31]).unwrap();
        source.write_ram(&[], (2 * MIB) as u64, &[0x42]).unwrap();
        source.write_ram(&[], (4 * MIB - 1) as u64, &[0x43]).unwrap();
        source.write_ram(&[], (4 * MIB) as u64, &[0x51]).unwrap();
        source.write_ram(&[], (5 * MIB - 1) as u64, &[0x52]).unwrap();

        let (geometry, residency) = snapshot_map(&source);
        assert_eq!(geometry.guest_len, (5 * MIB) as u64);
        assert_eq!(geometry.block_size, (2 * MIB) as u64);
        assert_eq!(
            residency,
            vec![
                MemorySnapshotResidency::Swapped,
                MemorySnapshotResidency::Swapped,
                MemorySnapshotResidency::Resident { slot: 0 },
            ]
        );

        // Preserve block zero, then expose a sparse EOF in swapped block one.
        // Snapshot output must include the logical zero tail, not short-read.
        unsafe {
            (&mut *source.inherited_memory_stub.overflow_file.get())
                .set_len((2 * MIB + 1) as u64)
                .unwrap();
        }
        let mut image = tempfile::tempfile().unwrap();
        for guest_block in 0..geometry.num_blocks {
            source
                .inherited_memory_stub
                .write_snapshot_block(guest_block, &mut image)
                .unwrap();
        }
        assert_eq!(image.metadata().unwrap().len(), (5 * MIB) as u64);
        image.seek(SeekFrom::Start(0)).unwrap();

        let mut restored = BxMemC::new(
            BxMemoryStubC::create_and_init(5 * MIB, 2 * MIB, 2 * MIB).unwrap(),
            false,
        );
        restored.set_a20_mask(u64::MAX);
        restored
            .inherited_memory_stub
            .actual_vector_mut()[MIB..2 * MIB]
            .fill(0xa5);
        for (guest_block, saved) in residency.iter().copied().enumerate() {
            restored
                .inherited_memory_stub
                .read_snapshot_block(guest_block as u32, saved, &mut image)
                .unwrap();
        }
        restored
            .inherited_memory_stub
            .finish_snapshot_restore(geometry, &residency)
            .unwrap();
        assert!(restored.inherited_memory_stub.actual_vector_slice()[MIB..2 * MIB]
            .iter()
            .all(|&byte| byte == 0));

        let mut value = [0];
        assert_eq!(restored.read_ram(&[], 0, &mut value).unwrap(), 1);
        assert_eq!(value, [0x31]);
        assert_eq!(
            restored.read_ram(&[], (2 * MIB) as u64, &mut value).unwrap(),
            1
        );
        assert_eq!(value, [0x42]);
        assert_eq!(
            restored
                .read_ram(&[], (4 * MIB - 1) as u64, &mut value)
                .unwrap(),
            1
        );
        assert_eq!(value, [0]);
        assert_eq!(
            restored.read_ram(&[], (4 * MIB) as u64, &mut value).unwrap(),
            1
        );
        assert_eq!(value, [0x51]);
        assert_eq!(
            restored
                .read_ram(&[], (5 * MIB - 1) as u64, &mut value)
                .unwrap(),
            1
        );
        assert_eq!(value, [0x52]);
    }

    #[test]
    fn snapshot_rejects_malformed_geometry_before_metadata_commit() {
        let mut memory = swapped_memory();
        let (geometry, residency) = snapshot_map(&memory);
        let mut malformed = geometry;
        malformed.guest_len += 1;

        let error = memory
            .inherited_memory_stub
            .finish_snapshot_restore(malformed, &residency)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(memory.inherited_memory_stub.snapshot_geometry(), geometry);
        assert_eq!(snapshot_map(&memory).1, residency);
    }

    #[test]
    fn snapshot_rejects_duplicate_slots_and_used_count_mismatch() {
        let mut memory = swapped_memory();
        let (geometry, before) = snapshot_map(&memory);
        let duplicate_slots = [
            MemorySnapshotResidency::Resident { slot: 0 },
            MemorySnapshotResidency::Resident { slot: 0 },
            MemorySnapshotResidency::Swapped,
            MemorySnapshotResidency::Swapped,
        ];
        let mut duplicate_geometry = geometry;
        duplicate_geometry.used_blocks = 2;
        assert_eq!(
            memory
                .inherited_memory_stub
                .finish_snapshot_restore(duplicate_geometry, &duplicate_slots)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(snapshot_map(&memory).1, before);

        let used_count_mismatch = [
            MemorySnapshotResidency::Resident { slot: 0 },
            MemorySnapshotResidency::Swapped,
            MemorySnapshotResidency::Swapped,
            MemorySnapshotResidency::Swapped,
        ];
        assert_eq!(
            memory
                .inherited_memory_stub
                .finish_snapshot_restore(geometry, &used_count_mismatch)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(snapshot_map(&memory).1, before);
    }

    #[test]
    fn snapshot_rejects_sparse_unique_resident_slots() {
        let mut memory = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, 3 * MIB, MIB).unwrap(),
            false,
        );
        memory.set_a20_mask(u64::MAX);
        let (mut geometry, before) = snapshot_map(&memory);
        geometry.used_blocks = 2;
        let sparse_map = [
            MemorySnapshotResidency::Resident { slot: 0 },
            MemorySnapshotResidency::Swapped,
            MemorySnapshotResidency::Resident { slot: 2 },
            MemorySnapshotResidency::Swapped,
        ];

        assert_eq!(
            memory
                .inherited_memory_stub
                .finish_snapshot_restore(geometry, &sparse_map)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(snapshot_map(&memory).1, before);
    }

    #[test]
    fn snapshot_rejects_truncated_swapped_block_input() {
        let mut memory = swapped_memory();
        let (geometry, before) = snapshot_map(&memory);
        let mut input = ShortReader {
            remaining: MIB - 1,
        };
        let error = memory
            .inherited_memory_stub
            .read_snapshot_block(0, MemorySnapshotResidency::Swapped, &mut input)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(memory.inherited_memory_stub.snapshot_geometry(), geometry);
        assert_eq!(snapshot_map(&memory).1, before);
    }

    #[test]
    fn typed_physical_access_crosses_subpage_guest_blocks() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut mem = BxMemC::new(
                    BxMemoryStubC::create_and_init(MIB, MIB, 1024).unwrap(),
                    false,
                );
                mem.set_a20_mask(u64::MAX);
                let cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                let pins = [CpuTlbPin::new(&*cpu)];
                let mut written = [0x11, 0x22, 0x33, 0x44];
                mem.write_physical_page(
                    &pins,
                    CpuMemoryPolicy::default(),
                    1022,
                    written.len(),
                    &mut written,
                )
                .unwrap();

                let mut read = [0; 4];
                mem.read_physical_page(
                    &pins,
                    CpuMemoryPolicy::default(),
                    1022,
                    read.len(),
                    &mut read,
                )
                .unwrap();
                assert_eq!(read, written);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn cpu_tlb_pin_sidecar_refreshes_and_clears_without_cpu_probe() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                let mut mem = BxMemC::new(
                    BxMemoryStubC::create_and_init(MIB, MIB, MIB).unwrap(),
                    false,
                );
                let pin = CpuTlbPin::new(&*cpu);
                let pins = core::slice::from_ref(&pin);

                cpu.wire_memory_access(core::ptr::NonNull::from(&mut mem), pins, &pin);
                assert!(!pin.is_range_pinned(0x4000, 0x5000));

                // A batch refresh publishes a mapping installed before the
                // execution scope; no allocator check dereferences `cpu`.
                let entry = &mut cpu.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page_addr = 0x4000;
                cpu.refresh_tlb_pin(&pin);
                assert!(pin.is_range_pinned(0x4000, 0x5000));

                // The CPU's real invalidation path synchronously removes the
                // slot, so stale over-pinning does not survive the scope.
                cpu.tlb_flush();
                assert!(!pin.is_range_pinned(0x4000, 0x5000));

                cpu.clear_memory_access();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn sibling_tlb_pin_blocks_loader_eviction() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut mem = BxMemC::new(
                    BxMemoryStubC::create_and_init(2 * MIB, MIB, MIB).unwrap(),
                    false,
                );
                mem.set_a20_mask(u64::MAX);
                let mut sibling = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                mem.write_ram(&[], 0, &[0x5a]).unwrap();
                let pins = [CpuTlbPin::new(&*sibling)];
                let host_ptr = mem
                    .get_host_mem_addr_pinned(
                        0,
                        MemoryAccessType::Read,
                        &pins,
                        CpuMemoryPolicy::default(),
                    )
                    .unwrap()
                    .unwrap()
                    .as_ptr() as usize;
                let entry = &mut sibling.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page_addr = host_ptr as _;
                sibling.refresh_tlb_pin(&pins[0]);

                assert!(matches!(
                    mem.load_RAM(&pins, &[0xa5], MIB as u64),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                let mut retained = [0];
                assert_eq!(mem.read_ram(&pins, 0, &mut retained).unwrap(), 1);
                assert_eq!(retained, [0x5a]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn fetch_window_pin_blocks_data_evicting_code_block() {
        // Bochs cpu.cc prefetch: `eipFetchPtr` stays valid until the next
        // refill. A sub-block fetch window has no ITLB slot, so the pin's
        // dedicated window interval is the only thing preventing a data
        // access from evicting the code block under a full swap cap.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut mem = BxMemC::new(
                    BxMemoryStubC::create_and_init(4 * MIB, MIB, MIB).unwrap(),
                    false,
                );
                mem.set_a20_mask(u64::MAX);
                let cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                let pins = [CpuTlbPin::new(&*cpu)];

                // Make guest block 0 resident and locate its host span.
                mem.write_ram(&[], 0, &[0x5a]).unwrap();
                let host = mem
                    .get_host_mem_addr_pinned(
                        0,
                        MemoryAccessType::Execute,
                        &pins,
                        CpuMemoryPolicy::default(),
                    )
                    .unwrap()
                    .unwrap()
                    .as_ptr() as usize;

                // Publish a bounded sub-block fetch window inside block 0,
                // exactly as `sync_fetch_window_pin` would.
                pins[0].set_fetch_window(host + 0x100, 0x80);
                assert!(pins[0].is_range_pinned(host, host + MIB));

                // A data access to a swapped block must NOT evict the pinned
                // code block — the sole victim candidate is protected.
                let mut buf = [0u8; 1];
                assert!(matches!(
                    mem.read_ram(&pins, 2 * MIB as u64, &mut buf),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
                // The code block is untouched: the stale-pointer scenario is
                // prevented at its root.
                let mut retained = [0];
                assert_eq!(mem.read_ram(&pins, 0, &mut retained).unwrap(), 1);
                assert_eq!(retained, [0x5a]);

                // Clearing the window releases the block for eviction.
                pins[0].set_fetch_window(0, 0);
                assert_eq!(mem.read_ram(&pins, 2 * MIB as u64, &mut buf).unwrap(), 1);
                assert_eq!(buf, [0]);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn pinned_cross_block_failure_returns_committed_copy_prefix() {
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut mem = BxMemC::new(
                    BxMemoryStubC::create_and_init(2 * MIB, MIB, MIB).unwrap(),
                    false,
                );
                mem.set_a20_mask(u64::MAX);
                let start = MIB as u64 - 2;
                assert_eq!(mem.write_ram(&[], start, &[0x11, 0x22]).unwrap(), 2);

                let mut sibling = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
                let pins = [CpuTlbPin::new(&*sibling)];
                let resident_base = mem
                    .get_host_mem_addr_pinned(
                        0,
                        MemoryAccessType::Read,
                        &pins,
                        CpuMemoryPolicy::default(),
                    )
                    .unwrap()
                    .unwrap()
                    .as_ptr() as usize;
                let entry = &mut sibling.dtlb.entries[0];
                entry.lpf = 0;
                entry.host_page_addr = resident_base as _;
                sibling.refresh_tlb_pin(&pins[0]);
                assert!(pins[0].is_range_pinned(resident_base, resident_base + MIB));

                let mut read = [0xcc; 4];
                assert_eq!(mem.read_ram(&pins, start, &mut read).unwrap(), 2);
                assert_eq!(read, [0x11, 0x22, 0xcc, 0xcc]);

                assert_eq!(
                    mem.write_ram(&pins, start, &[0x33, 0x44, 0x55, 0x66])
                        .unwrap(),
                    2
                );
                let mut committed = [0; 2];
                assert_eq!(mem.read_ram(&pins, start, &mut committed).unwrap(), 2);
                assert_eq!(committed, [0x33, 0x44]);

                let mut unavailable = [0; 1];
                assert!(matches!(
                    mem.read_ram(&pins, MIB as u64, &mut unavailable),
                    Err(Error::Memory(MemoryError::InsufficientRam))
                ));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    fn debugger_reference_crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in bytes {
            for bit in 0..8 {
                let feedback = ((crc ^ (u32::from(byte) >> bit)) & 1) != 0;
                crc >>= 1;
                if feedback {
                    crc ^= 0xEDB8_8320;
                }
            }
        }
        crc
    }

    #[cfg(any(feature = "bx_debugger", feature = "bx_gdb_stub"))]
    #[test]
    fn debugger_set_crc_cross_swapped_blocks() {
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(2 * MIB, MIB, MIB).unwrap(),
            false,
        );
        mem.set_a20_mask(u64::MAX);

        let start = (MIB - 47) as u64;
        let data: Vec<u8> = (0..128).map(|byte| byte as u8 ^ 0xA5).collect();
        assert!(mem
            .dbg_set_mem(&[], start, data.len() as u32, &data)
            .unwrap());

        let mut copied = vec![0; data.len()];
        assert_eq!(mem.read_ram(&[], start, &mut copied).unwrap(), data.len());
        assert_eq!(copied, data);

        let mut crc = 0;
        assert!(mem
            .dbg_crc32(&[], start, start + data.len() as u64 - 1, &mut crc)
            .unwrap());
        assert_eq!(crc, debugger_reference_crc32(&data));

        let mut rejected_crc = 0xA5A5_5A5A;
        assert!(!mem
            .dbg_crc32(
                &[],
                BX_PCI_HOLE_START,
                BX_PCI_HOLE_START,
                &mut rejected_crc,
            )
            .unwrap());
        assert_eq!(rejected_crc, 0xA5A5_5A5A);
    }

    #[test]
    fn failed_reload_leaves_target_block_swapped_until_retry() {
        let mut mem = swapped_memory();
        mem.write_ram(&[], 0, &[0x5a]).unwrap();
        unsafe {
            (&mut *mem.inherited_memory_stub.overflow_file.get())
                .set_len(0)
                .unwrap();
        }

        let mut byte = [0];
        assert!(mem.read_ram(&[], MIB as u64, &mut byte).is_err());
        let blocks = unsafe { &*mem.inherited_memory_stub.blocks_offsets.get() };
        assert!(matches!(blocks[1], super::Block::SwappedOut));

        unsafe {
            (&mut *mem.inherited_memory_stub.overflow_file.get())
                .set_len((2 * MIB) as u64)
                .unwrap();
        }
        assert_eq!(mem.read_ram(&[], MIB as u64, &mut byte).unwrap(), 1);
        assert_eq!(byte, [0]);
    }
}
