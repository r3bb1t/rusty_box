
mod error;
pub(crate) mod memory_rusty_box;
pub mod memory_stub;
pub mod misc_mem;
pub mod mmio;
pub mod mmio_map;
pub mod permissions;
pub(crate) mod plan;
mod residency;

#[cfg(test)]
mod tests;

pub use super::error::Result;
use crate::config::BxPhyAddress;
use residency::{Residency, ResidentParts};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
pub use error::*;

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

/// The three SMRAM control bits, read back together.
///
/// A named struct rather than the `(bool, bool, bool)` this replaces (doctrine
/// R0): all three are booleans, so nothing but position told them apart, and
/// position is exactly what an assertion gets wrong silently.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SmramState {
    /// SMRAM exists at all (Bochs `smram_available`).
    pub available: bool,
    /// SMRAM window open — the chipset's DOPEN bit.
    pub enabled: bool,
    /// SMRAM visible only in SMM — the chipset's DCLS bit.
    pub restricted: bool,
}

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
    /// Which guest block occupies which slot of the resident region, the
    /// dimensions that map is indexed by, and the overflow file the rest of
    /// the guest's RAM lives in — see `residency::Residency`.
    residency: Residency,
    /// The guest's memory bytes. Owned data rather than a raw pointer, so
    /// `Send` derives — see `RamBacking`.
    ///
    /// Laid out as the resident region (`Residency::resident_len` bytes from
    /// `vector_offset`), then the ROM image, then the bogus page. The resident
    /// region is what `Residency` is handed whenever it has to move bytes.
    backing: memory_stub::RamBacking,
    /// Bytes of `backing` this machine actually uses. The owned case rounds its
    /// allocation up to a whole number of pages, so the backing may be slightly
    /// longer; every accessor is bounded by this instead.
    actual_vector_len: usize,
    /// aligned correctly
    vector_offset: usize,
    /// 512k BIOS rom space + 128k expansion rom space
    rom_offset: usize,
    /// 4k for unexisting memory
    bogus_offset: usize,

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

}

// `Send` is derived, not promised: every field is owned or borrowed data. The
// hand-written `unsafe impl` this replaces asserted that the raw backing
// pointer was never aliased — true, but a claim the compiler could not check.
// The `Drop` that freed that pointer is gone too; `Box<[GuestPage]>` frees
// itself with the alignment it was allocated with.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<BxMemoryStubC>();
};

/// Fully-resident guest RAM for tests that need somewhere for a device to move
/// bytes. Nothing swaps, so a test observes only what it wrote.
#[cfg(test)]
pub(crate) fn test_ram() -> BxMemC {
    const BYTES: usize = 4 * 1024 * 1024;
    let mut memory = BxMemC::new(
        BxMemoryStubC::create_and_init(BYTES, BYTES, 1024 * 1024).expect("test guest RAM"),
        false,
    );
    memory.set_a20_mask(u64::MAX);
    memory
}


/// What a physical access still needs from its caller.
///
/// Bochs calls a device's handler from inside `writePhysicalPage`, which is why
/// this port carried raw device pointers on the memory subsystem. Memory now
/// answers *whose* the address is and stops; the caller — which owns the
/// devices — performs the dispatch. `#[must_use]` is what keeps that honest: a
/// caller that ignores the outcome silently drops every MMIO access.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysAccess {
    /// Memory serviced the access completely.
    Done,
    /// The address belongs to a memory-mapped device, which has not run yet.
    /// Carries where in that device's window it landed, so the dispatcher
    /// never has to subtract a base it would have to learn from the device.
    Mmio(mmio_map::MmioHit),
}

//#define BIOS_MAP_LAST128K(addr) (((addr) | 0xfff00000) & BIOS_MASK)

pub(crate) const BIOS_ROM_LOWER: u8 = 0x01;
pub(crate) const BIOS_ROM_EXTENDED: u8 = 0x02;
pub(crate) const BIOS_ROM_1MEG: u8 = 0x04;

#[derive(Debug)]
pub struct BxMemC {
    pub(crate) mmio: mmio_map::MmioMap,
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

}

impl BxMemC {
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

    /// Snapshot of SMRAM control state: available, enable/DOPEN,
    /// restricted/DCLS. A read-back for tests — the actual A0000-BFFFF routing
    /// decision is made directly against these flags in misc_mem.rs's host
    /// mapping and physical read/write paths, untouched here.
    #[cfg(test)]
    pub(crate) fn smram_state(&self) -> SmramState {
        SmramState {
            available: self.smram_available,
            enabled: self.smram_enable,
            restricted: self.smram_restricted,
        }
    }

    /// The PAM-derived memory type of a shadow RAM area. Mirrors
    /// `set_memory_type`: `area` is one of the 13 memory areas (C0000..F0000),
    /// `rw` is 0 = read path, 1 = write path.
    #[cfg(test)]
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

    /// Read back the current BIOS-write-enable state.
    #[cfg(test)]
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

}

// implement getters and setters for memory stub
impl BxMemoryStubC {
    /// Reconstruct the complete owned backing slice for memory-internal
    /// storage operations only. It is never a guest-linear RAM view.
    pub(super) fn actual_vector_slice(&self) -> &[u8] {
        &self.backing.as_slice()[..self.actual_vector_len]
    }

    /// Mutable counterpart to `actual_vector_slice`, restricted to memory
    /// internals such as ROM and resident-slot maintenance.
    pub(super) fn actual_vector_mut(&mut self) -> &mut [u8] {
        let len = self.actual_vector_len;
        &mut self.backing.as_mut_slice()[..len]
    }

    /// The resident region of the allocation, paired with the map that says
    /// which guest block occupies which slot of it.
    ///
    /// Handed out together because every residency change moves bytes: a
    /// swap-in reads a block from the overflow file into a slot, and the victim
    /// it displaces is written out of one. They are disjoint fields, so the
    /// borrow checker grants both at once — which is what lets `Residency`
    /// take the RAM as a parameter instead of owning it.
    ///
    /// The slice is exactly `Residency::resident_len` bytes. Construction
    /// establishes that the allocation holds that region plus the ROM image
    /// and the bogus page, and `vector_offset` is where it begins.
    fn resident_parts(&mut self) -> ResidentParts<'_> {
        let Self {
            residency,
            backing,
            vector_offset,
            ..
        } = self;
        let start = *vector_offset;
        let end = start + residency.resident_len();
        ResidentParts {
            ram: &mut backing.as_mut_slice()[start..end],
            map: residency,
        }
    }

    /// Guest RAM size in bytes. Could be > 4G.
    #[inline]
    pub(super) fn guest_len(&self) -> usize {
        self.residency.guest_len()
    }

    /// The current residency epoch. See `residency::Residency`'s `swap_epoch`.
    ///
    /// A consumer caching a RAM offset records this alongside it and discards
    /// the cache when the value changes. Reading is a plain load; under full
    /// residency the value is a constant zero.
    #[inline(always)]
    pub(crate) fn swap_epoch(&self) -> u64 {
        self.residency.swap_epoch()
    }

    /// Full O(num_blocks) identity-map scan — the ground truth behind the
    /// cached identity verdict, and the debug oracle in `identity_guest_base`.
    pub(super) fn scan_identity_map(&self) -> bool {
        self.residency.scan_identity_map()
    }

    /// The cached "host backing is a full identity map" verdict.
    #[inline]
    pub(super) fn is_identity_map(&self) -> bool {
        self.residency.is_identity_map()
    }

    pub(super) fn rom(&mut self) -> &mut [u8] {
        let ro = self.rom_offset;
        &mut self.actual_vector_mut()[ro..]
    }

    pub(super) fn bogus(&mut self) -> &mut [u8] {
        let bo = self.bogus_offset;
        &mut self.actual_vector_mut()[bo..]
    }


}
impl BxMemC {

    /// Copy RAM through the checked, block-resident backing store. This bypasses
    /// device handlers just like Bochs's physical DMA RAM path.
    pub(crate) fn read_ram(
        &mut self,
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
                self.inherited_memory_stub.guest_len(),
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
                let vector = match self.inherited_memory_stub.get_vector_offset(span.start) {
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
                self.inherited_memory_stub.guest_len(),
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
                let vector = match self.inherited_memory_stub.get_vector_offset(span.start) {
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

    pub(crate) fn get_memory_len(&self) -> usize {
        self.inherited_memory_stub.guest_len()
    }

    /// The residency epoch — bumped whenever a guest block changes slots, so a
    /// consumer holding cached RAM offsets can tell whether they still name the
    /// bytes they were filled from. See `BxMemoryStubC::swap_epoch`.
    #[inline(always)]
    pub(crate) fn swap_epoch(&self) -> u64 {
        self.inherited_memory_stub.swap_epoch()
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
            stub.is_identity_map(),
            stub.scan_identity_map(),
            "cached identity-map verdict diverged from the block table"
        );
        if !stub.is_identity_map() {
            return (core::ptr::null_mut(), 0);
        }
        let (guest_len, vector_offset) = (stub.guest_len(), stub.vector_offset);
        let stub = &mut self.inherited_memory_stub;
        // A returned pointer is fine — it is a FIELD holding one that would
        // block `Send`, and there is no longer any such field.
        let ptr = stub.backing.as_mut_slice()[vector_offset..].as_mut_ptr();
        (ptr, guest_len)
    }

    /// Base and length of the single allocation every direct host mapping
    /// lives in — guest RAM, the ROM image and the bogus page alike.
    ///
    /// Unlike `identity_guest_base` this stays valid when residency is partial,
    /// which is precisely why instruction fetch needs it: fetch also runs out
    /// of ROM, which is not guest RAM at all, and out of relocated RAM slots.
    /// A range from `host_mem_range` is an offset into exactly this.
    pub(crate) fn allocation_span(&mut self) -> (*mut u8, usize) {
        let stub = &mut self.inherited_memory_stub;
        let len = stub.actual_vector_len;
        (stub.backing.as_mut_slice().as_mut_ptr(), len)
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
    ) -> crate::snapshot::SnapResult<MemorySnapshotResidency> {
        self.inherited_memory_stub.snapshot_residency(guest_block)
    }

    #[cfg(feature = "std")]
    pub(crate) fn write_snapshot_block<W: crate::snapshot::SnapWrite>(
        &mut self,
        guest_block: u32,
        out: &mut W,
    ) -> crate::snapshot::SnapResult {
        self.inherited_memory_stub
            .write_snapshot_block(guest_block, out)
    }

    #[cfg(feature = "std")]
    pub(crate) fn read_snapshot_block<R: crate::snapshot::SnapRead>(
        &mut self,
        guest_block: u32,
        saved: MemorySnapshotResidency,
        input: &mut R,
    ) -> crate::snapshot::SnapResult {
        self.inherited_memory_stub
            .read_snapshot_block(guest_block, saved, input)
    }

    #[cfg(feature = "std")]
    pub(crate) fn finish_snapshot_restore(
        &mut self,
        geometry: MemorySnapshotGeometry,
        saved_map: &[MemorySnapshotResidency],
    ) -> crate::snapshot::SnapResult {
        self.inherited_memory_stub
            .finish_snapshot_restore(geometry, saved_map)
    }

    /// Count how many registered (non-None) memory handlers exist (for diagnostics).
    pub fn memory_handler_info(&self) -> usize {
        self.mmio.len()
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
impl BxMemC {
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
        memory_rusty_box::*, BxMemC, BxMemoryStubC, CpuMemoryPolicy, MemoryError,
        MemorySnapshotGeometry, MemorySnapshotResidency,
    };
    use std::io::{Seek, SeekFrom};
    use crate::Error;

    const MIB: usize = 1024 * 1024;

    fn swapped_memory() -> BxMemC {
        let mut memory = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, MIB, MIB).expect("memory allocation"),
            false,
        );
        memory.set_a20_mask(u64::MAX);
        memory
    }

    fn snapshot_map(memory: &BxMemC) -> (MemorySnapshotGeometry, Vec<MemorySnapshotResidency>) {
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

    /// A source that runs out part-way through a block, which is what a
    /// truncated snapshot looks like to the block reader.
    impl crate::snapshot::SnapRead for ShortReader {
        fn read_bytes(&mut self, out: &mut [u8]) -> crate::snapshot::SnapResult {
            if out.len() > self.remaining {
                return Err(crate::snapshot::SnapError::Truncated);
            }
            out.fill(0xa5);
            self.remaining -= out.len();
            Ok(())
        }
    }

    /// A block size that is not a power of two — zero included — is refused at
    /// construction. Zero is the case worth naming: it is the one value that
    /// divides the block count, so a machine configured with it must be turned
    /// away with an error rather than taken down by the arithmetic.
    #[test]
    fn a_block_size_that_is_not_a_power_of_two_is_refused() {
        for (guest, host, block_size) in [(0, 0, 0), (MIB, MIB, 0), (MIB, MIB, 3 * MIB)] {
            match BxMemoryStubC::create_and_init(guest, host, block_size) {
                Err(Error::Memory(MemoryError::BlockSizeIsNotAPowerOfTwo(reported))) => {
                    assert_eq!(reported, block_size)
                }
                other => panic!("{guest}/{host} with block size {block_size}: {other:?}"),
            }
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
            assert_eq!(mem.write_ram((block * MIB) as u64, &value).unwrap(), 1);
        }
        for block in 0..4usize {
            let mut value = [0];
            assert_eq!(mem.read_ram((block * MIB) as u64, &mut value).unwrap(), 1);
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

        assert_eq!(mem.write_ram(0, &[0x11]).unwrap(), 1);
        assert_eq!(mem.write_ram((2 * MIB) as u64, &[0x22]).unwrap(), 1);

        let mut first = [0];
        let mut second = [0];
        assert_eq!(mem.read_ram(0, &mut first).unwrap(), 1);
        assert_eq!(
            mem.read_ram((2 * MIB) as u64, &mut second).unwrap(),
            1
        );
        assert_eq!((first, second), ([0x11], [0x22]));
    }

    #[test]
    fn undersized_host_first_touch_reads_zero_without_eof() {
        let mut mem = swapped_memory();
        let mut bytes = [0xff; 32];
        assert_eq!(mem.read_ram((3 * MIB) as u64, &mut bytes).unwrap(), bytes.len());
        assert_eq!(bytes, [0; 32]);
    }

    #[test]
    fn block_aware_ram_copy_crosses_resident_and_swapped_blocks() {
        let mut mem = swapped_memory();
        let source = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(
            mem.write_ram((MIB - 2) as u64, &source).unwrap(),
            source.len()
        );
        let mut output = [0; 4];
        assert_eq!(
            mem.read_ram((MIB - 2) as u64, &mut output).unwrap(),
            output.len()
        );
        assert_eq!(output, source);
    }

    #[test]
    fn ram_copy_reports_out_of_range_without_partial_overflow() {
        let mut mem = swapped_memory();
        let data = [0xaa, 0xbb];
        assert_eq!(mem.write_ram((4 * MIB - 1) as u64, &data).unwrap(), 1);
        let mut last = [0];
        assert_eq!(mem.read_ram((4 * MIB - 1) as u64, &mut last).unwrap(), 1);
        assert_eq!(last, [0xaa]);
    }

    #[test]
    fn block_aware_ram_copy_reapplies_a20_across_one_megabyte() {
        let mut mem = swapped_memory();
        mem.set_a20_mask(0xFFFF_FFFF_FFEF_FFFF);
        assert_eq!(mem.write_ram(0x000f_fffe, &[1, 2, 3, 4]).unwrap(), 4);
        let mut low = [0; 2];
        let mut high = [0; 2];
        assert_eq!(mem.read_ram(0, &mut low).unwrap(), 2);
        assert_eq!(mem.read_ram(0x000f_fffe, &mut high).unwrap(), 2);
        assert_eq!(low, [3, 4]);
        assert_eq!(high, [1, 2]);
    }

    #[test]
    fn load_ram_above_host_backing_is_complete() {
        let mut mem = swapped_memory();
        let data = [7, 8, 9];
        mem.load_RAM(&data, (3 * MIB) as u64).unwrap();
        let mut output = [0; 3];
        assert_eq!(mem.read_ram((3 * MIB) as u64, &mut output).unwrap(), 3);
        assert_eq!(output, data);
        assert!(matches!(
            mem.load_RAM(&[1, 2], (4 * MIB - 1) as u64),
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
        source.write_ram(0, &[0x31]).unwrap();
        source.write_ram((2 * MIB) as u64, &[0x42]).unwrap();
        source.write_ram((4 * MIB - 1) as u64, &[0x43]).unwrap();
        source.write_ram((4 * MIB) as u64, &[0x51]).unwrap();
        source.write_ram((5 * MIB - 1) as u64, &[0x52]).unwrap();

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
        source
            .inherited_memory_stub
            .residency
            .overflow_file
            .set_len((2 * MIB + 1) as u64)
            .unwrap();
        let mut image = tempfile::tempfile().unwrap();
        {
            let mut sink = crate::snapshot::IoSink::new(&mut image);
            for guest_block in 0..geometry.num_blocks {
                source
                    .inherited_memory_stub
                    .write_snapshot_block(guest_block, &mut sink)
                    .unwrap();
            }
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
        {
            let mut stream = crate::snapshot::IoSource::new(&mut image);
            for (guest_block, saved) in residency.iter().copied().enumerate() {
                restored
                    .inherited_memory_stub
                    .read_snapshot_block(guest_block as u32, saved, &mut stream)
                    .unwrap();
            }
        }
        restored
            .inherited_memory_stub
            .finish_snapshot_restore(geometry, &residency)
            .unwrap();
        assert!(restored.inherited_memory_stub.actual_vector_slice()[MIB..2 * MIB]
            .iter()
            .all(|&byte| byte == 0));

        let mut value = [0];
        assert_eq!(restored.read_ram(0, &mut value).unwrap(), 1);
        assert_eq!(value, [0x31]);
        assert_eq!(
            restored.read_ram((2 * MIB) as u64, &mut value).unwrap(),
            1
        );
        assert_eq!(value, [0x42]);
        assert_eq!(
            restored
                .read_ram((4 * MIB - 1) as u64, &mut value)
                .unwrap(),
            1
        );
        assert_eq!(value, [0]);
        assert_eq!(
            restored.read_ram((4 * MIB) as u64, &mut value).unwrap(),
            1
        );
        assert_eq!(value, [0x51]);
        assert_eq!(
            restored
                .read_ram((5 * MIB - 1) as u64, &mut value)
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
        assert!(
            matches!(error, crate::snapshot::SnapError::Invalid(_)),
            "a rejected geometry names what was wrong: {error:?}"
        );
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
        assert!(
            matches!(
                memory
                    .inherited_memory_stub
                    .finish_snapshot_restore(duplicate_geometry, &duplicate_slots)
                    .unwrap_err(),
                crate::snapshot::SnapError::Invalid(_)
            )
        );
        assert_eq!(snapshot_map(&memory).1, before);

        let used_count_mismatch = [
            MemorySnapshotResidency::Resident { slot: 0 },
            MemorySnapshotResidency::Swapped,
            MemorySnapshotResidency::Swapped,
            MemorySnapshotResidency::Swapped,
        ];
        assert!(
            matches!(
                memory
                    .inherited_memory_stub
                    .finish_snapshot_restore(geometry, &used_count_mismatch)
                    .unwrap_err(),
                crate::snapshot::SnapError::Invalid(_)
            )
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

        assert!(
            matches!(
                memory
                    .inherited_memory_stub
                    .finish_snapshot_restore(geometry, &sparse_map)
                    .unwrap_err(),
                crate::snapshot::SnapError::Invalid(_)
            )
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
        assert_eq!(error, crate::snapshot::SnapError::Truncated);
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
                let mut written = [0x11, 0x22, 0x33, 0x44];
                let wrote = mem.write_physical_page(CpuMemoryPolicy::default(),
                    1022,
                    written.len(),
                    &mut written,
                )
                .unwrap();
                assert_eq!(wrote, super::PhysAccess::Done, "plain RAM reaches no device");

                let mut read = [0; 4];
                let got = mem.read_physical_page(CpuMemoryPolicy::default(),
                    1022,
                    read.len(),
                    &mut read,
                )
                .unwrap();
                assert_eq!(got, super::PhysAccess::Done, "plain RAM reaches no device");
                assert_eq!(read, written);
            })
            .unwrap()
            .join()
            .unwrap();
    }






    /// The residency epoch advances exactly when a guest block changes slots.
    ///
    /// This is the contract 4c relies on to retire the pin sidecar: pins stop
    /// eviction from stranding a cached RAM offset, an epoch instead lets
    /// eviction happen and tells the holder its offset died. If a relocation
    /// could ever go unannounced, a stale offset would silently read another
    /// block's bytes.
    /// A block may be evicted even while a CPU still references it.
    ///
    /// The pin sidecar used to veto exactly this: if any CPU held a direct
    /// reference into the candidate block, the allocator skipped it, and a
    /// machine whose live references covered every slot could not make progress
    /// at all — it returned `InsufficientRam` instead. The epoch replaces the
    /// veto with an announcement, so eviction always succeeds and the holder
    /// discards its cached offset at the next instruction fetch.
    ///
    /// Asserted as a guest-visible property (doctrine R9): the access that
    /// forces the eviction must SUCCEED and return the right bytes.
    #[test]
    fn a_referenced_block_can_still_be_evicted() {
        // One resident slot, three guest blocks: serving block 1 has no choice
        // but to evict block 0, which the previous read just referenced.
        let mut mem = swapped_memory();
        mem.write_ram(0, &[0xAA]).unwrap();
        mem.write_ram(MIB as u64, &[0xBB]).unwrap();

        let mut buf = [0u8; 1];
        mem.read_ram(0, &mut buf).unwrap();
        assert_eq!(buf, [0xAA]);
        let epoch_holding_block_0 = mem.swap_epoch();

        // Under the pin regime this could fail with InsufficientRam.
        mem.read_ram(MIB as u64, &mut buf)
            .expect("eviction must not be vetoed by an outstanding reference");
        assert_eq!(buf, [0xBB]);
        assert!(
            mem.swap_epoch() > epoch_holding_block_0,
            "the eviction must be announced so the stale offset is discarded"
        );
    }

    #[test]
    fn swap_epoch_advances_when_a_block_changes_slots() {
        // 4 MiB guest over 1 MiB host: one resident slot, so each access to a
        // different guest block must relocate.
        let mut mem = swapped_memory();
        let mut buf = [0u8; 1];

        mem.read_ram(0, &mut buf).unwrap();
        let after_first = mem.swap_epoch();

        mem.read_ram(MIB as u64, &mut buf).unwrap();
        let after_swap = mem.swap_epoch();
        assert!(
            after_swap > after_first,
            "relocating a block must announce itself: {after_first} -> {after_swap}"
        );

        mem.read_ram(2 * MIB as u64, &mut buf).unwrap();
        assert!(
            mem.swap_epoch() > after_swap,
            "every relocation advances the epoch, not just the first"
        );
    }

    /// Under full residency the epoch never moves, however much RAM is touched.
    ///
    /// The whole point of an epoch check on the hot path is that it compares
    /// against a value that never changes when nothing is swapping — which is
    /// every default configuration, and the only regime `no_alloc` has. A bump
    /// on some non-relocating path would be invisible to correctness tests and
    /// would quietly flush caches forever, so it is asserted here directly.
    #[test]
    fn swap_epoch_never_moves_under_full_residency() {
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(4 * MIB, 4 * MIB, MIB).expect("memory allocation"),
            false,
        );
        mem.set_a20_mask(u64::MAX);
        let start = mem.swap_epoch();

        let mut buf = [0u8; 4];
        for block in 0..4u64 {
            let base = block * MIB as u64;
            mem.write_ram(base, &[1, 2, 3, 4]).unwrap();
            mem.read_ram(base, &mut buf).unwrap();
            assert_eq!(buf, [1, 2, 3, 4]);
            // Straddle the block boundary too — the split path is a separate
            // walk through the block table.
            if block + 1 < 4 {
                mem.write_ram(base + MIB as u64 - 2, &[9, 9, 9, 9])
                    .unwrap();
            }
        }

        assert_eq!(
            mem.swap_epoch(),
            start,
            "no block relocated, so nothing may invalidate cached offsets"
        );
    }

    /// Guest bytes survive a full eviction round-trip through the swap file.
    ///
    /// The swap file used to be reached through an `UnsafeCell`, so the paths
    /// that write a victim out and read a block back in aliased it rather than
    /// borrowing it. This asserts the guest-visible consequence (doctrine R9) —
    /// what a block holds after being swapped out and back — instead of how the
    /// file happens to be borrowed, so it stays meaningful as that seam moves
    /// into `Residency`.
    #[test]
    fn evicted_blocks_keep_their_bytes_across_the_swap_file() {
        // Guest 4 MiB over 1 MiB of host RAM: one resident block, so touching a
        // second guest block must evict the first.
        let mut mem = swapped_memory();
        const B0: u64 = 0;
        const B1: u64 = MIB as u64;
        const B2: u64 = 2 * MIB as u64;

        mem.write_ram(B0, &[0x11, 0x22]).unwrap();
        mem.write_ram(B1, &[0x33, 0x44]).unwrap();
        mem.write_ram(B2, &[0x55, 0x66]).unwrap();

        // Each read below forces the block back in, evicting the previous one.
        let mut buf = [0u8; 2];
        mem.read_ram(B0, &mut buf).unwrap();
        assert_eq!(buf, [0x11, 0x22], "block 0 lost its bytes across eviction");
        mem.read_ram(B1, &mut buf).unwrap();
        assert_eq!(buf, [0x33, 0x44], "block 1 lost its bytes across eviction");
        mem.read_ram(B2, &mut buf).unwrap();
        assert_eq!(buf, [0x55, 0x66], "block 2 lost its bytes across eviction");

        // And a second full pass, proving the round-trip is repeatable rather
        // than surviving only the first write-out.
        mem.read_ram(B0, &mut buf).unwrap();
        assert_eq!(buf, [0x11, 0x22]);
    }

    #[test]
    fn failed_reload_leaves_target_block_swapped_until_retry() {
        let mut mem = swapped_memory();
        mem.write_ram(0, &[0x5a]).unwrap();
        mem.inherited_memory_stub
            .residency
            .overflow_file
            .set_len(0)
            .unwrap();

        let mut byte = [0];
        assert!(mem.read_ram(MIB as u64, &mut byte).is_err());
        let blocks = mem.inherited_memory_stub.residency.blocks();
        assert!(matches!(blocks[1], super::Block::SwappedOut));

        mem.inherited_memory_stub
            .residency
            .overflow_file
            .set_len((2 * MIB) as u64)
            .unwrap();
        assert_eq!(mem.read_ram(MIB as u64, &mut byte).unwrap(), 1);
        assert_eq!(byte, [0]);
    }
}
